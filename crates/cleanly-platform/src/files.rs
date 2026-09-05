//! Descriptor-relative quarantine. No recursive remove_dir_all, no cross-device copy fallback.
use cleanly_core::{Cancellation, Fingerprint, Result, validate_absolute};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::CString,
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
fn io(e: std::io::Error) -> String {
    e.to_string()
}
fn cstr(path: &Path) -> Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| "NUL in path".into())
}
pub fn fingerprint(meta: &fs::Metadata) -> Fingerprint {
    Fingerprint {
        device: meta.dev(),
        inode: meta.ino(),
        mode: meta.mode(),
        size: meta.size(),
        mtime: meta.mtime(),
        mtime_ns: meta.mtime_nsec(),
        ctime: meta.ctime(),
        ctime_ns: meta.ctime_nsec(),
        links: meta.nlink(),
    }
}
pub fn stat(path: &Path) -> Result<Fingerprint> {
    fs::symlink_metadata(path)
        .map(|m| fingerprint(&m))
        .map_err(io)
}
/// Open every directory component without following symlinks. Held fd anchors subsequent operations.
pub fn open_dir(path: &Path) -> Result<File> {
    validate_absolute(path)?;
    let mut dir = File::open("/").map_err(io)?;
    for part in path.components().skip(1) {
        let name = CString::new(part.as_os_str().as_bytes()).map_err(|_| "Invalid path")?;
        // SAFETY: name is NUL-terminated and parent fd remains live; successful openat returns an owned fd.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "Unsafe or inaccessible directory {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        dir = unsafe { File::from_raw_fd(fd) };
    }
    Ok(dir)
}
fn anchored(dir: &File, name: &Path) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", dir.as_raw_fd())).join(name)
}
fn child_dir(dir: &File, name: &Path) -> Result<File> {
    let name = cstr(name)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tree {
    pub mount_id: u64,
    pub entries: BTreeMap<PathBuf, Fingerprint>,
    pub bytes: u64,
}
pub fn snapshot(path: &Path, cancel: &Cancellation) -> Result<Tree> {
    validate_absolute(path)?;
    let parent = open_dir(path.parent().ok_or("Missing parent")?)?;
    let name = Path::new(path.file_name().ok_or("Missing filename")?);
    snapshot_at(&parent, name, cancel)
}
fn snapshot_at(parent: &File, name: &Path, cancel: &Cancellation) -> Result<Tree> {
    let mut tree = Tree {
        mount_id: mount_id(parent, name)?,
        entries: BTreeMap::new(),
        bytes: 0,
    };
    let start = Instant::now();
    walk(parent, name, Path::new(""), &mut tree, start, cancel, 0)?;
    Ok(tree)
}
fn walk(
    parent: &File,
    name: &Path,
    relative: &Path,
    tree: &mut Tree,
    start: Instant,
    cancel: &Cancellation,
    depth: usize,
) -> Result<()> {
    cancel.check()?;
    if depth > 64 || tree.entries.len() >= 100_000 || start.elapsed() > Duration::from_secs(10) {
        return Err("File inspection limit reached; cleanup is disabled for this tree".into());
    }
    if mount_id(parent, name)? != tree.mount_id {
        return Err("Mount boundary inside cleanup tree".into());
    }
    let meta = fs::symlink_metadata(anchored(parent, name)).map_err(io)?;
    if meta.file_type().is_symlink()
        || (!meta.is_dir() && !meta.is_file())
        || meta.uid() != unsafe { libc::geteuid() }
        || (!meta.is_dir() && meta.nlink() != 1)
    {
        return Err(
            "Symlink, special, foreign-owned or hard-linked file: preserve this tree".into(),
        );
    }
    let before = fingerprint(&meta);
    if let Some(root) = tree.entries.get(Path::new(""))
        && root.device != before.device
    {
        return Err("Mount boundary inside candidate".into());
    }
    if meta.is_file() {
        tree.bytes = tree.bytes.saturating_add(meta.len());
    }
    tree.entries.insert(relative.into(), before.clone());
    if meta.is_dir() {
        let dir = child_dir(parent, name)?;
        if fingerprint(&dir.metadata().map_err(io)?) != before {
            return Err("Directory changed while opening".into());
        }
        for entry in fs::read_dir(anchored(&dir, Path::new(""))).map_err(io)? {
            let entry = entry.map_err(io)?;
            let leaf = PathBuf::from(entry.file_name());
            walk(
                &dir,
                &leaf,
                &relative.join(&leaf),
                tree,
                start,
                cancel,
                depth + 1,
            )?;
        }
        if fingerprint(&dir.metadata().map_err(io)?) != before {
            return Err("Directory changed during inspection".into());
        }
    }
    if stat(&anchored(parent, name))? != before {
        return Err("File changed during inspection".into());
    }
    Ok(())
}
pub fn validate_target(home: &Path, path: &Path, appimage: bool, app_id: &str) -> Result<()> {
    validate_absolute(home)?;
    validate_absolute(path)?;
    if home.parent().is_none()
        || home == Path::new("/home")
        || path == home
        || !path.starts_with(home)
    {
        return Err("Cleanup target outside home".into());
    }
    let allowed = if appimage {
        ["Applications", ".local/bin", "Downloads"]
            .iter()
            .any(|p| path.parent() == Some(home.join(p).as_path()))
    } else {
        cleanly_core::valid_app_id(app_id) && path == home.join(".var/app").join(app_id)
    };
    if !allowed {
        return Err("Protected location or unproven cleanup target".into());
    }
    // Respect explicit XDG document locations too. Parse only literal quoted paths; never evaluate shell.
    let user_dirs = home.join(".config/user-dirs.dirs");
    if user_dirs.symlink_metadata().is_ok() {
        let text = read_regular(&user_dirs, 64 * 1024)?;
        for line in text.lines() {
            if let Some((key, value)) = line.split_once('=') {
                if ![
                    "XDG_DOCUMENTS_DIR",
                    "XDG_DESKTOP_DIR",
                    "XDG_PICTURES_DIR",
                    "XDG_VIDEOS_DIR",
                    "XDG_MUSIC_DIR",
                ]
                .contains(&key.trim())
                {
                    continue;
                }
                let raw = value.trim().trim_matches('"');
                let protected = if let Some(relative) = raw.strip_prefix("$HOME/") {
                    home.join(relative)
                } else if raw == "$HOME" {
                    home.into()
                } else {
                    PathBuf::from(raw)
                };
                validate_absolute(&protected)?;
                if path.starts_with(&protected) {
                    return Err("Explicit XDG user documents are protected".into());
                }
            }
        }
    }
    Ok(())
}
fn private_child(parent: &File, name: &str) -> Result<File> {
    let c = CString::new(name).map_err(|_| "Invalid component")?;
    let rc = unsafe { libc::mkdirat(parent.as_raw_fd(), c.as_ptr(), 0o700) };
    if rc < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let dir = child_dir(parent, Path::new(name))?;
    let m = dir.metadata().map_err(io)?;
    if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err("Cleanly storage must be owned by you with permissions 0700".into());
    }
    Ok(dir)
}
pub fn storage(home: &Path) -> Result<File> {
    let mut dir = open_dir(home)?;
    for name in [".local", "share"] {
        let c = CString::new(name).unwrap();
        let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), c.as_ptr(), 0o700) };
        if rc < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
            return Err(std::io::Error::last_os_error().to_string());
        }
        dir = child_dir(&dir, Path::new(name))?;
        let m = dir.metadata().map_err(io)?;
        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o022 != 0 {
            return Err("Unsafe XDG data container".into());
        }
    }
    private_child(&dir, "cleanly")
}
pub fn write_new(dir: &File, name: &str, data: &[u8]) -> Result<()> {
    if name.contains('/') || name == "." || name == ".." {
        return Err("Invalid record name".into());
    }
    let name = CString::new(name).map_err(|_| "Invalid name")?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut f = unsafe { File::from_raw_fd(fd) };
    f.write_all(data).map_err(io)?;
    f.sync_all().map_err(io)?;
    dir.sync_all().map_err(io)
}
pub fn read_record(dir: &File, name: &str) -> Result<String> {
    if name.contains('/') || name == "." || name == ".." {
        return Err("Invalid record name".into());
    }
    let name = CString::new(name).map_err(|_| "Invalid name")?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let f = unsafe { File::from_raw_fd(fd) };
    let m = f.metadata().map_err(io)?;
    if !m.is_file()
        || m.nlink() != 1
        || m.mode() & 0o077 != 0
        || m.uid() != unsafe { libc::geteuid() }
        || m.len() > 16 * 1024 * 1024
    {
        return Err("Unsafe or oversized record".into());
    }
    let mut text = String::new();
    f.take(16 * 1024 * 1024 + 1)
        .read_to_string(&mut text)
        .map_err(io)?;
    Ok(text)
}
pub fn operation_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuarantineRecord {
    pub id: String,
    pub original: PathBuf,
    pub app_id: String,
    pub appimage: bool,
    pub tree: Tree,
    pub created: u64,
}
fn rename_noreplace(from: &File, source: &Path, to: &File, dest: &Path) -> Result<()> {
    let source = cstr(source)?;
    let dest = cstr(dest)?;
    let rc = unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            source.as_ptr(),
            to.as_raw_fd(),
            dest.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc != 0 {
        Err(format!(
            "Atomic move refused (no overwrite/copy fallback): {}",
            std::io::Error::last_os_error()
        ))
    } else {
        from.sync_all().map_err(io)?;
        to.sync_all().map_err(io)
    }
}
fn equal_after_rename(before: &Tree, after: &Tree) -> bool {
    let mut expected = before.clone();
    if let (Some(a), Some(b)) = (
        expected.entries.get_mut(Path::new("")),
        after.entries.get(Path::new("")),
    ) {
        a.ctime = b.ctime;
        a.ctime_ns = b.ctime_ns;
    }
    expected == *after
}
pub fn quarantine(
    home: &Path,
    path: &Path,
    app_id: &str,
    appimage: bool,
    expected: &Tree,
    cancel: &Cancellation,
) -> Result<QuarantineRecord> {
    validate_target(home, path, appimage, app_id)?;
    let root = storage(home)?;
    let q = private_child(&root, "quarantine")?;
    let parent = open_dir(path.parent().ok_or("Missing parent")?)?;
    let name = Path::new(path.file_name().ok_or("Missing filename")?);
    if snapshot_at(&parent, name, cancel)? != *expected {
        return Err("Stale removal plan: tree changed".into());
    }
    let id = operation_id();
    let op = private_child(&q, &id)?;
    let record = QuarantineRecord {
        id: id.clone(),
        original: path.into(),
        app_id: app_id.into(),
        appimage,
        tree: expected.clone(),
        created: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    write_new(
        &op,
        "record.toml",
        toml::to_string(&record)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )?;
    cancel.check()?;
    rename_noreplace(&parent, name, &op, Path::new("payload"))?;
    match snapshot_at(&op, Path::new("payload"), cancel) {
        Ok(after) if equal_after_rename(expected, &after) => {
            write_new(&op, "committed", b"quarantined")?;
            Ok(record)
        }
        _ => {
            let rollback = rename_noreplace(&op, Path::new("payload"), &parent, name);
            Err(format!(
                "Tree changed during move; rollback: {rollback:?}. Inspect quarantine {id}."
            ))
        }
    }
}
pub fn quarantine_records(home: &Path) -> Result<Vec<QuarantineRecord>> {
    let root = storage(home)?;
    let q = private_child(&root, "quarantine")?;
    let mut records = Vec::new();
    for entry in fs::read_dir(anchored(&q, Path::new(""))).map_err(io)? {
        let entry = entry.map_err(io)?;
        let op = child_dir(&q, Path::new(&entry.file_name()))?;
        if let Ok(text) = read_record(&op, "record.toml") {
            let record: QuarantineRecord = toml::from_str(&text).map_err(|e| e.to_string())?;
            if fs::symlink_metadata(anchored(&op, Path::new("payload"))).is_ok() {
                records.push(record);
            }
        }
    }
    Ok(records)
}
pub fn restore(home: &Path, id: &str) -> Result<()> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit() || b == b'-') {
        return Err("Invalid operation ID".into());
    }
    let root = storage(home)?;
    let q = private_child(&root, "quarantine")?;
    let op = child_dir(&q, Path::new(id))?;
    let record: QuarantineRecord =
        toml::from_str(&read_record(&op, "record.toml")?).map_err(|e| e.to_string())?;
    if record.id != id {
        return Err("Mismatched quarantine identity".into());
    }
    validate_target(home, &record.original, record.appimage, &record.app_id)?;
    let tree = snapshot_at(&op, Path::new("payload"), &Cancellation::default())?;
    if !equal_after_rename(&record.tree, &tree) {
        return Err("Quarantined data changed; automatic restore refused".into());
    }
    let parent = open_dir(record.original.parent().ok_or("Missing parent")?)?;
    rename_noreplace(
        &op,
        Path::new("payload"),
        &parent,
        Path::new(record.original.file_name().ok_or("Missing filename")?),
    )?;
    write_new(&op, "restored", b"restored")
}
/// Atomically replace a record inside private storage. A destination symlink is replaced, never followed.
pub fn replace_record(dir: &File, name: &str, data: &[u8]) -> Result<()> {
    if name.contains('/') || name == "." || name == ".." {
        return Err("Invalid record name".into());
    }
    let temp = format!("{}.tmp", operation_id());
    write_new(dir, &temp, data)?;
    let from = CString::new(temp).map_err(|_| "Invalid temporary name")?;
    let to = CString::new(name).map_err(|_| "Invalid record name")?;
    let rc =
        unsafe { libc::renameat(dir.as_raw_fd(), from.as_ptr(), dir.as_raw_fd(), to.as_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    dir.sync_all().map_err(io)
}
/// Read a bounded regular file without following a symlink or blocking on a substituted FIFO.
pub fn read_regular(path: &Path, limit: u64) -> Result<String> {
    let file = open_regular(path)?;
    if file.metadata().map_err(io)?.len() > limit {
        return Err("Metadata size limit exceeded".into());
    }
    let mut text = String::new();
    file.take(limit + 1).read_to_string(&mut text).map_err(io)?;
    if text.len() as u64 > limit {
        return Err("Metadata grew beyond size limit".into());
    }
    Ok(text)
}
pub fn open_regular(path: &Path) -> Result<File> {
    validate_absolute(path)?;
    let parent = open_dir(path.parent().ok_or("Missing parent")?)?;
    let name = cstr(Path::new(path.file_name().ok_or("Missing filename")?))?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata().map_err(io)?.is_file() {
        return Err("Not a regular file".into());
    }
    Ok(file)
}

fn mount_id(parent: &File, name: &Path) -> Result<u64> {
    let name = cstr(name)?;
    let mut info = std::mem::MaybeUninit::<libc::statx>::zeroed();
    // SAFETY: valid parent descriptor, NUL-terminated name and appropriately sized writable statx buffer.
    let rc = unsafe {
        libc::statx(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW | libc::AT_NO_AUTOMOUNT,
            libc::STATX_MNT_ID,
            info.as_mut_ptr(),
        )
    };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let info = unsafe { info.assume_init() };
    if info.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err("Kernel cannot verify mount boundaries; cleanup disabled".into());
    }
    Ok(info.stx_mnt_id)
}
/// Best-effort bounded diagnostics. Records contain generated event fields, never file contents.
pub fn append_audit(dir: &File, data: &[u8]) -> Result<()> {
    if data.len() > 4096 {
        return Err("Audit event too large".into());
    }
    let name = c"events.toml";
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY
                | libc::O_APPEND
                | libc::O_CREAT
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let mut file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(io)?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.nlink() != 1
        || meta.mode() & 0o077 != 0
        || meta.len() > 1024 * 1024
    {
        return Err("Audit log unsafe or full".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("Audit log busy".into());
    }
    file.write_all(data).map_err(io)
}
