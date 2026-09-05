use cleanly_core::Cancellation;
use cleanly_platform::{CommandRunner, Runner, desktop, files::*};
use std::{
    fs,
    os::unix::fs::symlink,
    path::PathBuf,
    time::{Duration, Instant},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("cleanly-test-{}", operation_id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn app(&self) -> PathBuf {
        let path = self.0.join(".var/app/org.example.App");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("settings"), "private").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn protected_containers_and_documents() {
    let f = Fixture::new();
    for path in [
        PathBuf::from("/"),
        "/usr".into(),
        "/etc".into(),
        f.0.clone(),
        f.0.join(".config"),
        f.0.join(".local"),
        f.0.join("Documents/Example"),
        f.0.join(".var/app/org.other.App"),
    ] {
        assert!(validate_target(&f.0, &path, false, "org.example.App").is_err());
    }
}
#[test]
fn another_apps_config_is_kept() {
    let f = Fixture::new();
    let p = f.0.join(".config/Example");
    fs::create_dir_all(&p).unwrap();
    assert!(validate_target(&f.0, &p, false, "org.example.App").is_err());
    assert!(p.exists());
}
#[test]
fn root_symlink_rejected() {
    let f = Fixture::new();
    let p = f.app();
    symlink(&p, f.0.join("link")).unwrap();
    assert!(snapshot(&f.0.join("link"), &Cancellation::default()).is_err());
}
#[test]
fn nested_symlink_rejected() {
    let f = Fixture::new();
    let p = f.app();
    symlink("/etc", p.join("outside")).unwrap();
    assert!(snapshot(&p, &Cancellation::default()).is_err());
}
#[test]
fn ancestor_symlink_rejected() {
    let f = Fixture::new();
    let p = f.app();
    symlink(p.parent().unwrap(), f.0.join("alias")).unwrap();
    assert!(snapshot(&f.0.join("alias/org.example.App"), &Cancellation::default()).is_err());
}
#[test]
fn hardlinked_data_is_kept() {
    let f = Fixture::new();
    let p = f.app();
    fs::hard_link(p.join("settings"), f.0.join("shared")).unwrap();
    assert!(snapshot(&p, &Cancellation::default()).is_err());
}
#[test]
fn stale_file_cannot_be_quarantined() {
    let f = Fixture::new();
    let p = f.app();
    let c = Cancellation::default();
    let tree = snapshot(&p, &c).unwrap();
    fs::write(p.join("settings"), "changed data").unwrap();
    assert!(quarantine(&f.0, &p, "org.example.App", false, &tree, &c).is_err());
    assert!(p.exists());
}
#[test]
fn quarantine_restore_roundtrip() {
    let f = Fixture::new();
    let p = f.app();
    let c = Cancellation::default();
    let tree = snapshot(&p, &c).unwrap();
    let r = quarantine(&f.0, &p, "org.example.App", false, &tree, &c).unwrap();
    assert!(!p.exists());
    assert_eq!(quarantine_records(&f.0).unwrap().len(), 1);
    restore(&f.0, &r.id).unwrap();
    assert_eq!(fs::read_to_string(p.join("settings")).unwrap(), "private");
    assert!(quarantine_records(&f.0).unwrap().is_empty());
}
#[test]
fn restore_never_overwrites() {
    let f = Fixture::new();
    let p = f.app();
    let c = Cancellation::default();
    let tree = snapshot(&p, &c).unwrap();
    let r = quarantine(&f.0, &p, "org.example.App", false, &tree, &c).unwrap();
    fs::create_dir(&p).unwrap();
    fs::write(p.join("new"), "keep").unwrap();
    assert!(restore(&f.0, &r.id).is_err());
    assert!(p.join("new").exists());
}
#[test]
fn private_storage_symlink_rejected() {
    let f = Fixture::new();
    fs::create_dir_all(f.0.join(".local/share")).unwrap();
    symlink("/tmp", f.0.join(".local/share/cleanly")).unwrap();
    assert!(storage(&f.0).is_err());
}
#[test]
fn cancelled_inspection_stops() {
    let f = Fixture::new();
    let c = Cancellation::default();
    c.cancel();
    assert!(snapshot(&f.app(), &c).is_err());
}
#[test]
fn desktop_exec_is_only_metadata() {
    let text =
        "[Desktop Entry]\nType=Application\nName=Evil\nExec=sh -c 'touch /tmp/cleanly-pwned'\n";
    let entry = desktop::parse(text).unwrap();
    assert!(desktop::exact_executable(&entry.exec).is_none());
}
#[test]
fn desktop_duplicate_and_oversize_rejected() {
    assert!(desktop::parse("[Desktop Entry]\nType=Application\nName=A\nName=B").is_err());
    assert!(desktop::parse(&"a".repeat(128 * 1024 + 1)).is_err());
}
#[test]
fn bounded_process_and_cancellation() {
    let runner = CommandRunner {
        timeout: Duration::from_millis(100),
        limit: 64,
    };
    let start = Instant::now();
    assert!(
        runner
            .run("/usr/bin/sleep", &["5"], &Cancellation::default())
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(
        runner
            .run("/usr/bin/yes", &[], &Cancellation::default())
            .is_err()
    );
    let c = Cancellation::default();
    c.cancel();
    assert!(runner.run("/usr/bin/true", &[], &c).is_err());
}
#[test]
fn arguments_are_literal_and_failures_reported() {
    let runner = CommandRunner::default();
    let out = runner
        .run(
            "/usr/bin/printf",
            &["%s", "$(touch /tmp/never); --all"],
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(out.stdout, "$(touch /tmp/never); --all");
    assert!(
        !runner
            .run("/usr/bin/false", &[], &Cancellation::default())
            .unwrap()
            .success
    );
}
#[test]
fn explicit_xdg_documents_override_is_protected() {
    let f = Fixture::new();
    let path = f.app();
    fs::create_dir_all(f.0.join(".config")).unwrap();
    fs::write(
        f.0.join(".config/user-dirs.dirs"),
        "XDG_DOCUMENTS_DIR=\"$HOME/.var/app/org.example.App\"\n",
    )
    .unwrap();
    assert!(validate_target(&f.0, &path, false, "org.example.App").is_err());
}
#[test]
fn metadata_symlink_and_fifo_are_never_read() {
    let f = Fixture::new();
    symlink("/etc/passwd", f.0.join("evil.desktop")).unwrap();
    assert!(desktop::read(&f.0.join("evil.desktop")).is_err());
    use std::os::unix::ffi::OsStrExt;
    let path = f.0.join("fifo.desktop");
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    assert!(desktop::read(&path).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}
