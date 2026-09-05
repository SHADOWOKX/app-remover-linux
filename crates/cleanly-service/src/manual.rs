//! Unmanaged launchers are visible but inspection-only. Executable reference is not ownership.
use cleanly_core::*;
use cleanly_platform::{Runner, desktop, files};
use std::{path::PathBuf, sync::Arc};
pub struct Manual {
    pub home: PathBuf,
    pub runner: Arc<dyn Runner>,
}
impl PackageBackend for Manual {
    fn kind(&self) -> Backend {
        Backend::Manual
    }
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let mut apps = Vec::new();
        for dir in [
            self.home.join(".local/share/applications"),
            PathBuf::from("/usr/local/share/applications"),
        ] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.take(10_000).flatten() {
                cancel.check()?;
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "desktop") {
                    continue;
                }
                let Ok(desktop) = desktop::read(&path) else {
                    continue;
                };
                if desktop.flatpak.is_some() || desktop.snap.is_some() {
                    continue;
                }
                let Some(id) = path.to_str() else {
                    continue;
                };
                if !id.contains(['*', '?', '[', '\\'])
                    && std::path::Path::new("/usr/bin/dpkg-query").exists()
                {
                    let out = self
                        .runner
                        .run("/usr/bin/dpkg-query", &["-S", "--", id], cancel)?;
                    if out.success {
                        continue;
                    }
                }
                apps.push(InstalledApp{id:id.into(),name:desktop.name,version:"Unknown".into(),backend:Backend::Manual,scope:"unmanaged".into(),architecture:"Unknown".into(),publisher:"Unknown".into(),location:path,icon:desktop.icon,size:None,protection:Some("Manual installation: launcher metadata does not prove exclusive ownership. Removal disabled.".into())});
            }
        }
        Ok(apps)
    }
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        cancel.check()?;
        let desktop = desktop::read(&app.location)?;
        let mut current = app.clone();
        current.name = desktop.name;
        current.protection = Some(
            "Ownership is unproven. Cleanly will keep the launcher, executable, settings and data."
                .into(),
        );
        Ok(AppManifest{app:current,backend_token:"inspection-only".into(),files:vec![RemovalCandidate{path:app.location.clone(),ownership:OwnershipEvidence::Protected{reason:"A desktop launcher describes how to start an application. It does not prove ownership of its executable or files.".into()},confidence:Confidence::Unknown,category:FileCategory::Protected,size:files::stat(&app.location).ok().map(|f|f.size),selected_by_default:false,fingerprint:None}],notes:vec![format!("Exec is metadata only and was never executed: {}",desktop.exec)]})
    }
}
