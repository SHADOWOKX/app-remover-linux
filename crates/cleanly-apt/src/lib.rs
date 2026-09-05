use cleanly_core::*;
use cleanly_platform::{Runner, desktop, files};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
pub const FORMAT: &str = "${binary:Package}\t${Version}\t${Architecture}\t${Installed-Size}\t${Essential}\t${Protected}\t${Priority}\t${Section}\t${db:Status-Status}\n";
pub struct Apt {
    pub runner: Arc<dyn Runner>,
}
pub fn protected(
    id: &str,
    essential: &str,
    protected: &str,
    priority: &str,
    section: &str,
) -> Option<String> {
    let name = id.split(':').next().unwrap_or(id);
    if essential == "yes" || protected == "yes" || matches!(priority, "required" | "important") {
        return Some(
            "System component: dpkg Essential/Protected or required/important priority".into(),
        );
    }
    if section == "kernel"
        || section == "libs"
        || [
            "linux-",
            "grub",
            "systemd",
            "libc6",
            "libgcc",
            "libstdc++",
            "gnome-shell",
            "gnome-session",
            "gdm",
            "lightdm",
            "sddm",
            "xserver",
            "xwayland",
            "mutter",
            "network-manager",
            "netplan",
            "wpasupplicant",
            "dbus",
            "apt",
            "dpkg",
            "sudo",
            "polkit",
            "policykit",
            "ubuntu-desktop",
            "ubuntu-minimal",
            "ubuntu-standard",
            "cleanly",
        ]
        .iter()
        .any(|p| name.starts_with(p))
    {
        return Some(
            "System component: boot, desktop, networking, privilege or package infrastructure"
                .into(),
        );
    }
    None
}
pub fn parse_packages(text: &str) -> Result<HashMap<String, InstalledApp>> {
    let mut result = HashMap::new();
    for line in text.lines() {
        let p: Vec<_> = line.split('\t').collect();
        if p.len() != 9 {
            return Err("Unexpected dpkg metadata schema".into());
        }
        if p[8] != "installed" {
            continue;
        }
        if !valid_package(p[0]) {
            return Err("Invalid dpkg package identity".into());
        }
        result.insert(
            p[0].into(),
            InstalledApp {
                id: p[0].into(),
                name: p[0].into(),
                version: p[1].into(),
                backend: Backend::Apt,
                scope: "system".into(),
                architecture: p[2].into(),
                publisher: "Unknown".into(),
                location: "/".into(),
                icon: "application-x-executable-symbolic".into(),
                size: p[3].parse::<u64>().ok().and_then(|n| n.checked_mul(1024)),
                protection: protected(p[0], p[4], p[5], p[6], p[7]),
            },
        );
    }
    Ok(result)
}
pub fn parse_owners(text: &str) -> HashMap<PathBuf, Vec<String>> {
    let mut owners: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for line in text.lines() {
        if let Some((owner, path)) = line.split_once(": ") {
            if !path.starts_with('/') {
                continue;
            }
            let list: Vec<String> = owner
                .split(", ")
                .filter(|s| valid_package(s))
                .map(String::from)
                .collect();
            if !list.is_empty() {
                owners.entry(path.into()).or_default().extend(list);
            }
        }
    }
    for list in owners.values_mut() {
        list.sort();
        list.dedup();
    }
    owners
}
impl Apt {
    pub fn package(&self, id: &str, cancel: &Cancellation) -> Result<InstalledApp> {
        if !valid_package(id) {
            return Err("Invalid package ID".into());
        }
        let text = self
            .runner
            .run(
                "/usr/bin/dpkg-query",
                &["-W", &format!("-f={FORMAT}"), "--", id],
                cancel,
            )?
            .checked()?;
        parse_packages(&text)?
            .remove(id)
            .ok_or("Package is no longer installed".into())
    }
    pub fn dry_run(&self, id: &str, cancel: &Cancellation) -> Result<String> {
        if !valid_package(id) {
            return Err("Invalid package ID".into());
        }
        self.runner
            .run(
                "/usr/bin/dpkg",
                &[
                    "--log=/dev/null",
                    "--no-force-all",
                    "--dry-run",
                    "--remove",
                    "--",
                    id,
                ],
                cancel,
            )?
            .checked()
    }
}
impl PackageBackend for Apt {
    fn kind(&self) -> Backend {
        Backend::Apt
    }
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>> {
        let text = self
            .runner
            .run(
                "/usr/bin/dpkg-query",
                &["-W", &format!("-f={FORMAT}")],
                cancel,
            )?
            .checked()?;
        let packages = parse_packages(&text)?;
        let entries = std::fs::read_dir("/usr/share/applications").map_err(|e| e.to_string())?;
        let mut apps = HashMap::new();
        for file in entries.flatten() {
            cancel.check()?;
            let path = file.path();
            if path.extension().is_none_or(|s| s != "desktop") {
                continue;
            }
            let Ok(entry) = desktop::read(&path) else {
                continue;
            };
            let Some(path_str) = path.to_str() else {
                continue;
            };
            if path_str.contains(['*', '?', '[', '\\']) {
                continue;
            }
            let output = self
                .runner
                .run("/usr/bin/dpkg-query", &["-S", "--", path_str], cancel)?;
            let map = parse_owners(&output.stdout);
            let Some(owners) = map.get(&path) else {
                continue;
            };
            if owners.len() != 1 {
                continue;
            }
            if let Some(package) = packages.get(&owners[0]) {
                let mut app = package.clone();
                app.name = entry.name;
                app.icon = entry.icon;
                app.location = path;
                apps.entry(app.id.clone()).or_insert(app);
            }
        }
        let mut result: Vec<_> = apps.into_values().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(result)
    }
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest> {
        let current = self.package(&app.id, cancel)?;
        if current.version != app.version {
            return Err("Package changed; refresh application list".into());
        }
        let text = self
            .runner
            .run("/usr/bin/dpkg-query", &["-L", "--", &app.id], cancel)?
            .checked()?;
        let paths: Vec<_> = text.lines().filter(|s| s.starts_with('/')).collect();
        let mut owners = HashMap::new();
        for chunk in paths.chunks(100) {
            let mut args = vec!["-S", "--"];
            args.extend(
                chunk
                    .iter()
                    .copied()
                    .filter(|p| !p.contains(['*', '?', '[', '\\'])),
            );
            if args.len() > 2 {
                owners.extend(parse_owners(
                    &self
                        .runner
                        .run("/usr/bin/dpkg-query", &args, cancel)?
                        .stdout,
                ));
            }
        }
        let files = paths
            .iter()
            .map(|path| {
                let path = PathBuf::from(path);
                let registered = owners.get(&path).cloned().unwrap_or_default();
                let exclusive = registered == [app.id.clone()];
                let fp = files::stat(&path).ok();
                RemovalCandidate {
                    size: fp.as_ref().and_then(|f| {
                        if f.mode & lib_mode_mask() == 0o100000 {
                            Some(f.size)
                        } else {
                            None
                        }
                    }),
                    path,
                    ownership: OwnershipEvidence::PackageDatabase {
                        package: app.id.clone(),
                        owners: registered,
                    },
                    confidence: if exclusive {
                        Confidence::Verified
                    } else {
                        Confidence::Unknown
                    },
                    category: if exclusive {
                        FileCategory::Application
                    } else {
                        FileCategory::Protected
                    },
                    selected_by_default: false,
                    fingerprint: fp,
                }
            })
            .collect();
        let mut notes=vec!["Only this explicit package is requested. No autoremove, recursive dependency cleanup or manual deletion of package files. Configuration and personal data are preserved.".into(),"Installed size comes from dpkg metadata; it is an estimate, not measured freed space.".into()];
        let mut updated = app.clone();
        updated.protection = current.protection;
        updated.size = current.size;
        let launchers: Vec<_> = paths
            .iter()
            .filter(|p| p.ends_with(".desktop"))
            .filter_map(|p| desktop::read(std::path::Path::new(p)).ok().map(|d| d.name))
            .collect();
        if launchers.len() > 1 {
            notes.push(format!("This package provides several graphical launchers; removing it affects all of them: {}",launchers.join(", ")));
        }
        if updated.protection.is_none() {
            match self.dry_run(&app.id, cancel) {
                Ok(summary) => notes.push(summary),
                Err(e) => {
                    updated.protection =
                        Some(format!("Dependency/safety check refused removal: {e}"))
                }
            }
        }
        Ok(AppManifest {
            app: updated,
            files,
            notes,
            backend_token: format!("{}:{}", current.id, current.version),
        })
    }
}
fn lib_mode_mask() -> u32 {
    0o170000
}
