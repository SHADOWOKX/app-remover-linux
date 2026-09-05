use cleanly_core::*;
use cleanly_platform::{Runner, files};
use std::{path::PathBuf, sync::Arc};
pub struct Flatpak {
    pub runner: Arc<dyn Runner>,
    pub home: PathBuf,
}
pub fn valid_ref(s: &str) -> bool {
    let p: Vec<_> = s.split('/').collect();
    p.len() == 4
        && p[0] == "app"
        && valid_app_id(p[1])
        && p[2..].iter().all(|s| {
            !s.is_empty()
                && s.as_bytes()[0].is_ascii_alphanumeric()
                && s.len() < 100
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
}
pub fn parse_list(text: &str, scope: &str) -> Result<Vec<InstalledApp>> {
    let mut apps = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let p: Vec<_> = line.split('\t').collect();
        if p.len() != 6 {
            return Err("Unexpected Flatpak metadata schema".into());
        }
        let reference = if p[0].starts_with("app/") {
            p[0].to_string()
        } else {
            format!("app/{}", p[0])
        };
        if !valid_ref(&reference) {
            return Err("Invalid Flatpak application reference".into());
        }
        let parts: Vec<_> = reference.split('/').collect();
        apps.push(InstalledApp {
            id: reference.clone(),
            name: p[1].into(),
            version: if p[2].is_empty() {
                "Unknown".into()
            } else {
                p[2].into()
            },
            backend: Backend::Flatpak,
            scope: scope.into(),
            architecture: parts[2].into(),
            publisher: format!("Origin: {} (publisher unknown)", p[3]),
            location: PathBuf::new(),
            icon: parts[1].into(),
            size: parse_size(p[4]),
            protection: None,
        });
    }
    Ok(apps)
}
impl Flatpak {
    pub fn list(&self, scope: &str, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        if !matches!(scope, "user" | "system") {
            return Err("Unsupported installation scope".into());
        }
        let output = self
            .runner
            .run(
                "/usr/bin/flatpak",
                &[
                    "list",
                    &format!("--{scope}"),
                    "--app",
                    "--columns=ref,name,version,origin,size,installation",
                ],
                cancel,
            )?
            .checked()?;
        parse_list(&output, scope)
    }
    pub fn info(&self, app: &InstalledApp, flag: &str, cancel: &Cancellation) -> Result<String> {
        if !valid_ref(&app.id) || !matches!(app.scope.as_str(), "user" | "system") {
            return Err("Invalid Flatpak identity".into());
        }
        self.runner
            .run(
                "/usr/bin/flatpak",
                &["info", &format!("--{}", app.scope), flag, "--", &app.id],
                cancel,
            )?
            .checked()
            .map(|s| s.trim().into())
    }
}
impl PackageBackend for Flatpak {
    fn kind(&self) -> Backend {
        Backend::Flatpak
    }
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let mut apps = self.list("user", cancel)?;
        apps.extend(self.list("system", cancel)?);
        Ok(apps)
    }
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        let commit = self.info(app, "--show-commit", cancel)?;
        let location = self.info(app, "--show-location", cancel)?;
        let metadata = self.info(app, "--show-metadata", cancel)?;
        let mut updated = app.clone();
        updated.location = location.into();
        updated.size = self.info(app, "--show-size", cancel)?.parse::<u64>().ok();
        let id = app.id.split('/').nth(1).ok_or("Invalid ref")?;
        let all = self.discover(cancel)?;
        let mut exclusive = all
            .iter()
            .filter(|a| a.id.split('/').nth(1) == Some(id))
            .count()
            == 1;
        // Unknown additional system installations can share the same user sandbox ID.
        match std::fs::read_dir("/etc/flatpak/installations.d") {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    exclusive = false;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => exclusive = false,
        }
        if [
            "FLATPAK_SYSTEM_DIR",
            "FLATPAK_USER_DIR",
            "FLATPAK_SYSTEM_HELPER_ON_SESSION",
        ]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
        {
            exclusive = false;
        }
        // Explicit cross-application sandbox references are a preservation signal.
        for other in all
            .iter()
            .filter(|a| a.id != app.id || a.scope != app.scope)
        {
            match self.info(other, "--show-permissions", cancel) {
                Ok(permissions) => {
                    if permissions.contains(&format!(".var/app/{id}")) {
                        exclusive = false;
                    }
                }
                Err(_) => exclusive = false,
            }
        }
        let path = self.home.join(".var/app").join(id);
        let mut candidates = Vec::new();
        let mut notes=vec!["Only the exact application ref is requested; runtime cleanup and --delete-data are never used. Filesystem permissions granted outside the sandbox are preserved.".into(),format!("Flatpak metadata / permissions:\n{metadata}")];
        if path.symlink_metadata().is_ok() {
            let tree = files::snapshot(&path, cancel);
            let safe = exclusive && tree.is_ok();
            let fp = files::stat(&path).ok();
            if let Err(e) = &tree {
                notes.push(format!("Sandbox data protected: {e}"));
            }
            candidates.push(RemovalCandidate {
                path,
                ownership: OwnershipEvidence::FlatpakSandbox {
                    app_id: id.into(),
                    exclusive,
                },
                confidence: if safe {
                    Confidence::Strong
                } else {
                    Confidence::Unknown
                },
                category: if safe {
                    FileCategory::Data
                } else {
                    FileCategory::Protected
                },
                size: tree.as_ref().ok().map(|t| t.bytes),
                selected_by_default: safe,
                fingerprint: fp,
            });
        }
        // Deployment trees can be shared. Display package-manager metadata without claiming manual ownership.
        candidates.insert(0,RemovalCandidate{path:updated.location.clone(),ownership:OwnershipEvidence::Protected{reason:format!("Flatpak deployment for {} at commit {commit}. Managed only by Flatpak; shared objects/runtimes stay under its control.",app.id)},confidence:Confidence::Verified,category:FileCategory::Application,size:None,selected_by_default:false,fingerprint:None});
        notes.push("Sandbox data is quarantined as one unit; cache, config and state inside it are not independently inferred. Freed deployment storage is unknown because Flatpak shares objects.".into());
        Ok(AppManifest {
            app: updated,
            files: candidates,
            notes,
            backend_token: commit,
        })
    }
}

/// Flatpak's documented size column is display-formatted; retain it only as an estimate.
fn parse_size(text: &str) -> Option<u64> {
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let n = parts[0].parse::<f64>().ok()?;
    let factor = match parts[1] {
        "bytes" | "B" => 1.,
        "kB" | "KB" => 1000.,
        "MB" => 1_000_000.,
        "GB" => 1_000_000_000.,
        _ => return None,
    };
    if n.is_finite() && n >= 0. && (n * factor) < u64::MAX as f64 {
        Some((n * factor) as u64)
    } else {
        None
    }
}
