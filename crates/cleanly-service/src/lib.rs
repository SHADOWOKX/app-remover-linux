//! Application orchestration: backend isolation, immutable preview, revalidation and durable history.
use cleanly_core::*;
use cleanly_platform::{
    CommandRunner, Runner,
    files::{self, Tree},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
mod manual;
pub mod privileged;
#[derive(Clone)]
pub struct Service {
    pub home: PathBuf,
    pub runner: Arc<dyn Runner>,
    pub operations: Arc<dyn Runner>,
}
#[derive(Clone, Debug)]
pub struct Discovery {
    pub backend: Backend,
    pub apps: Vec<InstalledApp>,
    pub error: Option<String>,
}
#[derive(Clone, Debug)]
pub struct PreparedPlan {
    plan: RemovalPlan,
    trees: BTreeMap<usize, Tree>,
}
impl PreparedPlan {
    pub fn plan(&self) -> &RemovalPlan {
        &self.plan
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperationResult {
    pub id: String,
    #[serde(default)]
    pub timestamp: u64,
    pub app: String,
    pub backend: Backend,
    pub package_removed: bool,
    pub quarantined_bytes: u64,
    pub quarantined: Vec<String>,
    pub errors: Vec<String>,
    pub details: Vec<String>,
}
impl Service {
    pub fn new(home: PathBuf) -> Self {
        Self {
            home,
            runner: Arc::new(CommandRunner::default()),
            operations: Arc::new(CommandRunner {
                timeout: Duration::from_secs(600),
                limit: 8 * 1024 * 1024,
            }),
        }
    }
    pub fn backend(&self, kind: Backend) -> Result<Box<dyn PackageBackend>> {
        Ok(match kind {
            Backend::Apt => Box::new(cleanly_apt::Apt {
                runner: self.runner.clone(),
            }),
            Backend::Flatpak => Box::new(cleanly_flatpak::Flatpak {
                runner: self.runner.clone(),
                home: self.home.clone(),
            }),
            Backend::Snap => Box::new(cleanly_snap::Snap {
                runner: self.runner.clone(),
            }),
            Backend::AppImage => Box::new(cleanly_appimage::AppImage {
                home: self.home.clone(),
            }),
            Backend::Manual => Box::new(manual::Manual {
                home: self.home.clone(),
                runner: self.runner.clone(),
            }),
        })
    }
    pub fn discover(&self, callback: impl Fn(Discovery) + Send + Sync, cancel: &Cancellation) {
        std::thread::scope(|scope| {
            let callback = &callback;
            for backend in [
                Backend::Apt,
                Backend::Flatpak,
                Backend::Snap,
                Backend::AppImage,
                Backend::Manual,
            ] {
                scope.spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.backend(backend).and_then(|b| b.discover(cancel))
                    }))
                    .unwrap_or_else(|_| Err("Discovery worker failed unexpectedly".into()));
                    self.audit(
                        "discovery",
                        backend,
                        result.as_ref().map_or(0, |apps| apps.len()),
                        result.is_ok(),
                    );
                    callback(match result {
                        Ok(apps) => Discovery {
                            backend,
                            apps,
                            error: None,
                        },
                        Err(e) => Discovery {
                            backend,
                            apps: vec![],
                            error: Some(e),
                        },
                    });
                });
            }
        });
    }
    pub fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        let mut manifest = self.backend(app.backend)?.inspect(app, cancel)?;
        if app.backend == Backend::AppImage {
            // An AppImage signature cannot override a package database registration.
            let id = app
                .location
                .to_str()
                .ok_or("Non-UTF8 AppImage path is inspection-only")?;
            if id.contains(['*', '?', '[', '\\']) {
                manifest.app.protection = Some(
                    "Path contains package-query patterns; ownership cannot be checked exactly"
                        .into(),
                );
            } else if Path::new("/usr/bin/dpkg-query").exists() {
                let output = self
                    .runner
                    .run("/usr/bin/dpkg-query", &["-S", "--", id], cancel)?;
                let owners = cleanly_apt::parse_owners(&output.stdout);
                if owners.contains_key(&app.location) {
                    manifest.app.protection = Some(
                        "This AppImage is registered to dpkg; manual removal is prohibited".into(),
                    );
                } else if output.success
                    || !output
                        .stderr
                        .starts_with("dpkg-query: no path found matching pattern ")
                {
                    manifest.app.protection =
                        Some("Package ownership check was inconclusive".into());
                }
            } else {
                manifest.app.protection=Some("This distribution has no supported package ownership database; AppImage removal is disabled.".into());
            }
        }
        self.audit(
            "ownership_inspection",
            app.backend,
            manifest.files.len(),
            true,
        );
        Ok(manifest)
    }
    pub fn prepare(
        &self,
        manifest: AppManifest,
        mode: RemovalMode,
        selected: Vec<usize>,
        cancel: &Cancellation,
    ) -> Result<PreparedPlan> {
        let plan = RemovalPlan::build(manifest, mode, selected)?;
        let mut trees = BTreeMap::new();
        for &i in plan.selected() {
            let file = &plan.manifest().files[i];
            let (appimage, id) = match &file.ownership {
                OwnershipEvidence::AppImageIdentity { .. } => (true, ""),
                OwnershipEvidence::FlatpakSandbox { app_id, .. } => (false, app_id.as_str()),
                _ => return Err("Unsupported cleanup evidence".into()),
            };
            files::validate_target(&self.home, &file.path, appimage, id)?;
            let tree = files::snapshot(&file.path, cancel)?;
            if tree.entries.get(Path::new("")) != file.fingerprint.as_ref() {
                return Err("File changed since inspection; review again".into());
            }
            trees.insert(i, tree);
        }
        self.audit(
            "removal_plan",
            plan.manifest().app.backend,
            plan.selected().len(),
            true,
        );
        Ok(PreparedPlan { plan, trees })
    }
    fn revalidate(&self, prepared: &PreparedPlan, cancel: &Cancellation) -> Result<()> {
        let old = prepared.plan.manifest();
        let current = self.inspect(&old.app, cancel)?;
        if current.app.protection.is_some()
            || current.backend_token != old.backend_token
            || current.app.version != old.app.version
        {
            return Err("Application or safety state changed; refresh and review again".into());
        }
        for &i in prepared.plan.selected() {
            let old_file = &old.files[i];
            let now = current
                .files
                .iter()
                .find(|f| f.path == old_file.path)
                .ok_or("Ownership evidence disappeared")?;
            if now != old_file || !now.cleanup_allowed() {
                return Err("Cleanup ownership changed; review again".into());
            }
            if files::snapshot(&old_file.path, cancel)? != prepared.trees[&i] {
                return Err("Cleanup tree changed; review again".into());
            }
        }
        Ok(())
    }
    pub fn execute(
        &self,
        prepared: PreparedPlan,
        progress: impl Fn(&str),
        cancel: &Cancellation,
    ) -> Result<OperationResult> {
        progress("Validating removal plan");
        self.revalidate(&prepared, cancel)?;
        let plan = &prepared.plan;
        let manifest = plan.manifest();
        let app = &manifest.app;
        let id = files::operation_id();
        let storage = files::storage(&self.home)?;
        // An immutable journal is persisted BEFORE external mutations. Failed/incomplete requests remain auditable.
        files::write_new(
            &storage,
            &format!("{id}-plan.toml"),
            toml::to_string(plan).map_err(|e| e.to_string())?.as_bytes(),
        )?;
        let mut result = OperationResult {
 id:id.clone(), timestamp:std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(),
 app:app.name.clone(), backend:app.backend, package_removed:false, quarantined_bytes:0,
 quarantined:vec![], errors:vec![], details:vec!["User documents and unproven/shared leftovers preserved. Freed disk space: unknown (quarantine does not free space).".into()],
 };
        self.audit("execution", app.backend, plan.selected().len(), true);
        let command_result: Result<String> = if app.backend == Backend::AppImage {
            Ok(String::new())
        } else {
            progress("Removing package — keep Cleanly open");
            self.remove_package(manifest, cancel)
        };
        match command_result {
            Err(e) => result.errors.push(e),
            Ok(details) if !details.is_empty() => result.details.push(details),
            _ => {}
        }

        progress("Verifying package-manager state");
        if app.backend != Backend::AppImage {
            match self.is_removed(app, cancel) {
                Ok(value) => {
                    result.package_removed = value;
                    if !value {
                        result
                            .errors
                            .push("Package remains installed; cleanup skipped".into());
                    }
                }
                Err(e) => result
                    .errors
                    .push(format!("Could not verify removal: {e}. Cleanup skipped.")),
            }
        }
        if (result.package_removed || app.backend == Backend::AppImage) && result.errors.is_empty()
        {
            progress("Quarantining selected files");
            for &i in plan.selected() {
                let candidate = &manifest.files[i];
                let (image, app_id) = match &candidate.ownership {
                    OwnershipEvidence::AppImageIdentity { .. } => (true, ""),
                    OwnershipEvidence::FlatpakSandbox { app_id, .. } => (false, app_id.as_str()),
                    _ => unreachable!("validated selection"),
                };
                // Namespace must remain exclusive after package removal; a new installation must not lose its data.
                if app.backend == Backend::Flatpak {
                    let flatpak = cleanly_flatpak::Flatpak {
                        runner: self.runner.clone(),
                        home: self.home.clone(),
                    };
                    match flatpak.discover(cancel) {
                        Ok(apps)
                            if !apps.iter().any(|a| a.id.split('/').nth(1) == Some(app_id)) => {}
                        _ => {
                            result.errors.push("Flatpak ID is installed again or scope check failed; data preserved".into());
                            continue;
                        }
                    }
                }
                match files::quarantine(
                    &self.home,
                    &candidate.path,
                    app_id,
                    image,
                    &prepared.trees[&i],
                    cancel,
                ) {
                    Ok(record) => {
                        result.quarantined_bytes += record.tree.bytes;
                        result.quarantined.push(record.id);
                        if image {
                            result.package_removed = !candidate.path.exists();
                        }
                    }
                    Err(e) => result
                        .errors
                        .push(format!("{}: {e}", candidate.path.display())),
                }
            }
        }
        self.audit(
            "cleanup_result",
            app.backend,
            result.quarantined.len(),
            result.errors.is_empty(),
        );
        progress("Writing result");
        files::write_new(
            &storage,
            &format!("{id}-result.toml"),
            toml::to_string(&result)
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
        .map_err(|e| {
            format!("Operation completed but history could not be saved: {e}. Result: {result:?}")
        })?;
        Ok(result)
    }
    fn remove_package(&self, manifest: &AppManifest, cancel: &Cancellation) -> Result<String> {
        let app = &manifest.app;
        let runner = &self.operations;
        let output = match app.backend {
            Backend::Flatpak => {
                if !cleanly_flatpak::valid_ref(&app.id)
                    || !matches!(app.scope.as_str(), "user" | "system")
                {
                    return Err("Invalid Flatpak identity".into());
                }
                runner
                    .run(
                        "/usr/bin/flatpak",
                        &[
                            "uninstall",
                            &format!("--{}", app.scope),
                            "--app",
                            "--no-related",
                            "--noninteractive",
                            "--",
                            &app.id,
                        ],
                        cancel,
                    )?
                    .checked()?
            }
            Backend::Apt | Backend::Snap => {
                self.audit("privilege_request", app.backend, 1, true);
                privileged::verify_helper()?;
                let request = privileged::Request {
                    action: if app.backend == Backend::Apt {
                        privileged::Action::RemoveApt
                    } else {
                        privileged::Action::RemoveSnap
                    },
                    id: app.id.clone(),
                    version: app.version.clone(),
                    token: manifest.backend_token.clone(),
                };
                let request = toml::to_string(&request).map_err(|e| e.to_string())?;
                runner
                    .run("/usr/bin/pkexec", &[privileged::HELPER, &request], cancel)?
                    .checked()?
            }
            _ => return Err("Unsupported package operation".into()),
        };
        Ok(output)
    }
    fn is_removed(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<bool> {
        match app.backend {
            Backend::Apt => {
                let output = self.runner.run(
                    "/usr/bin/dpkg-query",
                    &["-W", "-f=${db:Status-Status}", "--", &app.id],
                    cancel,
                )?;
                if output.success {
                    Ok(matches!(
                        output.stdout.trim(),
                        "config-files" | "not-installed"
                    ))
                } else if output
                    .stderr
                    .starts_with("dpkg-query: no packages found matching ")
                {
                    Ok(true)
                } else {
                    Err(output.stderr)
                }
            }
            Backend::Flatpak => Ok(!cleanly_flatpak::Flatpak {
                runner: self.runner.clone(),
                home: self.home.clone(),
            }
            .list(&app.scope, cancel)?
            .iter()
            .any(|a| a.id == app.id)),
            Backend::Snap => Ok(!cleanly_snap::Snap {
                runner: self.runner.clone(),
            }
            .list(cancel)?
            .iter()
            .any(|a| a.id == app.id)),
            _ => Err("Unsupported verification".into()),
        }
    }
    pub fn history(&self) -> Result<Vec<OperationResult>> {
        let root = files::storage(&self.home)?;
        use std::os::fd::AsRawFd;
        let mut results: Vec<OperationResult> = Vec::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd()))
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with("-result.toml") {
                let text = files::read_record(&root, &name)?;
                results.push(toml::from_str(&text).map_err(|e| e.to_string())?);
            }
        }
        results.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(results)
    }
}
#[derive(Serialize, Deserialize)]
struct AppCache {
    apps: Vec<InstalledApp>,
}
impl Service {
    pub fn cached_apps(&self) -> Result<Vec<InstalledApp>> {
        let root = files::storage(&self.home)?;
        let text = files::read_record(&root, "applications.toml")?;
        let cache: AppCache = toml::from_str(&text).map_err(|e| e.to_string())?;
        Ok(cache.apps)
    }
    pub fn cache_apps(&self, apps: Vec<InstalledApp>) -> Result<()> {
        let root = files::storage(&self.home)?;
        files::replace_record(
            &root,
            "applications.toml",
            toml::to_string(&AppCache { apps })
                .map_err(|e| e.to_string())?
                .as_bytes(),
        )
    }
    pub fn appearance(&self) -> u32 {
        files::storage(&self.home)
            .and_then(|root| files::read_record(&root, "appearance"))
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|n| *n <= 2)
            .unwrap_or(0)
    }
    pub fn save_appearance(&self, value: u32) -> Result<()> {
        let root = files::storage(&self.home)?;
        files::replace_record(&root, "appearance", value.to_string().as_bytes())
    }
}
#[derive(Serialize)]
struct AuditEvent<'a> {
    timestamp: u64,
    stage: &'a str,
    backend: &'a str,
    count: usize,
    success: bool,
}
impl Service {
    fn audit(&self, stage: &str, backend: Backend, count: usize, success: bool) {
        let event = AuditEvent {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            stage,
            backend: backend.label(),
            count,
            success,
        };
        if let (Ok(root), Ok(record)) = (files::storage(&self.home), toml::to_string(&event)) {
            let _ = files::append_audit(&root, format!("[[events]]\n{record}\n").as_bytes());
        }
    }
}
