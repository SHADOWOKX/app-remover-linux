use cleanly_core::*;
use cleanly_platform::files;
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
pub struct AppImage {
    pub home: PathBuf,
}
pub fn identify(path: &Path) -> Result<Fingerprint> {
    validate_absolute(path)?;
    let _parent = files::open_dir(path.parent().ok_or("No parent")?)?;
    let mut file = files::open_regular(path)?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    let fp = files::fingerprint(&meta);
    if !meta.is_file() || fp.links != 1 {
        return Err("AppImage must be an exclusive regular file".into());
    }
    let mut header = [0; 11];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..4] != b"\x7fELF" || &header[8..10] != b"AI" || !matches!(header[10], 1 | 2) {
        return Err("Not an AppImage signature".into());
    }
    if files::fingerprint(&file.metadata().map_err(|e| e.to_string())?) != fp
        || files::stat(path)? != fp
    {
        return Err("AppImage changed during inspection".into());
    }
    Ok(fp)
}
impl PackageBackend for AppImage {
    fn kind(&self) -> Backend {
        Backend::AppImage
    }
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let mut apps = Vec::new();
        for relative in ["Applications", ".local/bin", "Downloads"] {
            let dir = self.home.join(relative);
            if files::open_dir(&dir).is_err() {
                continue;
            }
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.take(10_000).flatten() {
                cancel.check()?;
                let path = entry.path();
                let Ok(fp) = identify(&path) else {
                    continue;
                };
                let Some(id) = path.to_str() else {
                    continue;
                };
                apps.push(InstalledApp {
                    id: id.into(),
                    name: path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into(),
                    version: "Unknown".into(),
                    backend: Backend::AppImage,
                    scope: "user".into(),
                    architecture: "Unknown".into(),
                    publisher: "Unknown".into(),
                    location: path,
                    icon: "application-x-executable-symbolic".into(),
                    size: Some(fp.size),
                    protection: None,
                });
            }
        }
        Ok(apps)
    }
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        files::validate_target(&self.home, &app.location, true, "")?;
        let fp = identify(&app.location)?;
        cancel.check()?;
        let mut candidates = vec![RemovalCandidate {
            path: app.location.clone(),
            ownership: OwnershipEvidence::AppImageIdentity {
                executable: app.location.clone(),
            },
            confidence: Confidence::Verified,
            category: FileCategory::Application,
            size: Some(fp.size),
            selected_by_default: true,
            fingerprint: Some(fp.clone()),
        }];
        for relative in [".local/share/applications", ".config/autostart"] {
            let Ok(entries) = fs::read_dir(self.home.join(relative)) else {
                continue;
            };
            for entry in entries.take(10_000).flatten() {
                let path = entry.path();
                if let Ok(desktop) = cleanly_platform::desktop::read(&path)
                    && cleanly_platform::desktop::exact_executable(&desktop.exec).as_deref()
                        == app.location.to_str()
                {
                    candidates.push(RemovalCandidate {
                        path,
                        ownership: OwnershipEvidence::ExactLauncher {
                            executable: app.location.clone(),
                        },
                        confidence: Confidence::Weak,
                        category: FileCategory::Integration,
                        size: None,
                        selected_by_default: false,
                        fingerprint: None,
                    });
                }
            }
        }
        Ok(AppManifest{app:app.clone(),files:candidates,notes:vec!["The exact AppImage is moved to quarantine, not executed. No name-based configuration, icon or data matching. Matching launchers remain review-only because exclusivity is not established.".into()],backend_token:format!("{}:{}:{}:{}:{}",fp.device,fp.inode,fp.size,fp.mtime,fp.ctime)})
    }
}
