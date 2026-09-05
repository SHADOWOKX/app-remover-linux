use cleanly_core::*;
use cleanly_platform::Runner;
use std::{path::PathBuf, sync::Arc};
pub struct Snap {
    pub runner: Arc<dyn Runner>,
}
pub fn parse_list(text: &str) -> Result<Vec<InstalledApp>> {
    let mut lines = text.lines();
    if lines
        .next()
        .is_none_or(|h| !h.starts_with("Name ") && !h.starts_with("Name\t"))
    {
        return Err("Unexpected Snap list schema".into());
    }
    let mut apps = Vec::new();
    for line in lines {
        let p: Vec<_> = line.split_whitespace().collect();
        if p.len() != 6 || !valid_snap(p[0]) || !valid_revision(p[2]) {
            return Err("Unexpected Snap list row".into());
        }
        let protected = p[5].split(',').any(|n| matches!(n, "base" | "snapd"))
            || [
                "core",
                "bare",
                "snapd",
                "gnome-",
                "gtk-common-themes",
                "cups",
            ]
            .iter()
            .any(|s| p[0].starts_with(s));
        apps.push(InstalledApp {
            id: p[0].into(),
            name: p[0].into(),
            version: p[1].into(),
            backend: Backend::Snap,
            scope: "system".into(),
            architecture: "Unknown".into(),
            publisher: p[4].into(),
            location: PathBuf::from(format!("/snap/{}/{}", p[0], p[2])),
            icon: "application-x-executable-symbolic".into(),
            size: None,
            protection: protected.then(|| "System/base Snap: removal disabled".into()),
        });
    }
    Ok(apps)
}
impl Snap {
    pub fn list(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let out = self.runner.run("/usr/bin/snap", &["list"], cancel)?;
        if out.success {
            parse_list(&out.stdout)
        } else {
            Err(format!("Snap unavailable: {}", out.stderr.trim()))
        }
    }
}
impl PackageBackend for Snap {
    fn kind(&self) -> Backend {
        Backend::Snap
    }
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let apps = self.list(cancel)?;
        let dir =
            std::fs::read_dir("/var/lib/snapd/desktop/applications").map_err(|e| e.to_string())?;
        let mut ids = std::collections::HashMap::new();
        for file in dir.flatten() {
            if let Ok(entry) = cleanly_platform::desktop::read(&file.path())
                && let Some(id) = entry.snap
            {
                ids.entry(id).or_insert(entry.name);
            }
        }
        Ok(apps
            .into_iter()
            .filter_map(|mut app| {
                app.name = ids.get(&app.id)?.clone();
                Some(app)
            })
            .collect())
    }
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        if !valid_snap(&app.id) {
            return Err("Invalid Snap identity".into());
        }
        let current = self
            .list(cancel)?
            .into_iter()
            .find(|a| a.id == app.id)
            .ok_or("Snap is no longer installed")?;
        if current.location != app.location || current.version != app.version {
            return Err("Snap changed; refresh list".into());
        }
        let metadata = self
            .runner
            .run(
                "/usr/bin/snap",
                &[
                    "info",
                    "--",
                    app.location.to_str().ok_or("Invalid Snap location")?,
                ],
                cancel,
            )?
            .checked()?;
        let yaml_path = app.location.join("meta/snap.yaml");
        let yaml = cleanly_platform::files::read_regular(&yaml_path, 1024 * 1024)
            .map_err(|e| format!("Cannot verify installed Snap type: {e}"))?;
        if yaml.len() > 1024 * 1024 {
            return Err("Snap metadata too large".into());
        }
        let kind = yaml
            .lines()
            .find_map(|l| l.strip_prefix("type:"))
            .map(|s| s.trim().trim_matches(['\'', '"']))
            .unwrap_or("app");
        let fp = std::fs::metadata(format!(
            "/var/lib/snapd/snaps/{}_{}.snap",
            app.id,
            app.location
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ))
        .ok()
        .map(|m| m.len());
        let candidate = RemovalCandidate {
            path: app.location.clone(),
            ownership: OwnershipEvidence::Protected {
                reason: "Snap revision mount. Only snapd may remove it.".into(),
            },
            confidence: Confidence::Verified,
            category: FileCategory::Application,
            size: fp,
            selected_by_default: false,
            fingerprint: None,
        };
        let mut app = app.clone();
        app.protection = current.protection;
        if kind != "app" {
            app.protection = Some(format!("System Snap type {kind}: removal disabled"));
        }
        app.size = fp;
        Ok(AppManifest{backend_token:current.location.to_string_lossy().into(),app,files:vec![candidate],notes:vec![metadata,"Before removal Cleanly requires an explicit snapd data snapshot and verifies its integrity. Cleanly never uses --purge. Snapshots follow snapd exclusion policies and consume space; no manual Snap data cleanup. Restore using snap restore after reinstalling.".into()]})
    }
}

pub fn valid_revision(s: &str) -> bool {
    let digits = s.strip_prefix('x').unwrap_or(s);
    !digits.is_empty() && digits.len() < 20 && digits.bytes().all(|b| b.is_ascii_digit())
}
pub fn snapshot_set(text: &str, id: &str) -> Result<String> {
    for line in text.lines().skip(1) {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() >= 6
            && parts[1] == id
            && parts[0].bytes().all(|b| b.is_ascii_digit())
            && !parts[0].is_empty()
        {
            return Ok(parts[0].into());
        }
    }
    Err("Could not verify snapd snapshot set identity; removal refused".into())
}
