//! Minimal fixed-action helper. Never accepts executable paths, shell fragments or cleanup paths.
use cleanly_core::*;
use cleanly_platform::{CommandRunner, Runner};
use serde::{Deserialize, Serialize};
use std::{os::unix::fs::MetadataExt, path::Path, sync::Arc, time::Duration};
pub const HELPER: &str = "/usr/libexec/cleanly-helper";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    RemoveApt,
    RemoveSnap,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub action: Action,
    pub id: String,
    pub version: String,
    pub token: String,
}
impl Request {
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > 4096 {
            return Err("Oversized request".into());
        }
        let request: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        let valid = match request.action {
            Action::RemoveApt => valid_package(&request.id),
            Action::RemoveSnap => valid_snap(&request.id),
        };
        if !valid
            || request.version.is_empty()
            || request.version.len() > 256
            || request.token.len() > 512
            || request.version.chars().any(char::is_control)
            || request.token.chars().any(char::is_control)
        {
            return Err("Invalid structured removal request".into());
        }
        Ok(request)
    }
}
pub fn verify_helper() -> Result<()> {
    for path in ["/usr", "/usr/libexec", HELPER] {
        let m = std::fs::symlink_metadata(path).map_err(
            |_| "Administrator installation required: run sudo make install after building",
        )?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 || m.file_type().is_symlink() {
            return Err("Privileged helper or its parent is not root-owned and protected".into());
        }
    }
    if !Path::new(HELPER).is_file() {
        return Err("Privileged helper is not a regular file".into());
    }
    Ok(())
}
/// Caller must establish uid=0 and clear its environment before entering this function.
pub fn execute(request: Request) -> Result<()> {
    let cancel = Cancellation::default();
    let runner = Arc::new(CommandRunner {
        timeout: Duration::from_secs(540),
        limit: 8 * 1024 * 1024,
    });
    match request.action {
        Action::RemoveApt => {
            let apt = cleanly_apt::Apt {
                runner: runner.clone(),
            };
            let app = apt.package(&request.id, &cancel)?;
            if app.protection.is_some() {
                return Err("System package removal is prohibited".into());
            }
            if app.version != request.version
                || request.token != format!("{}:{}", app.id, app.version)
            {
                return Err("Package changed since review".into());
            }
            if !apt.discover(&cancel)?.iter().any(|a| a.id == request.id) {
                return Err("Package is not a verified graphical application".into());
            }
            apt.dry_run(&request.id, &cancel)?;
            runner
                .run(
                    "/usr/bin/dpkg",
                    &["--no-force-all", "--remove", "--", &request.id],
                    &cancel,
                )?
                .checked()?;
        }
        Action::RemoveSnap => {
            let snap = cleanly_snap::Snap {
                runner: runner.clone(),
            };
            let app = snap
                .discover(&cancel)?
                .into_iter()
                .find(|a| a.id == request.id)
                .ok_or("Not a graphical Snap")?;
            if app.protection.is_some()
                || app.version != request.version
                || app.location.to_str() != Some(&request.token)
            {
                return Err("Protected Snap or revision changed since review".into());
            }
            let inspected = snap.inspect(&app, &cancel)?;
            if inspected.app.protection.is_some() {
                return Err("Protected Snap type".into());
            }
            let saved = runner
                .run(
                    "/usr/bin/snap",
                    &["save", "--abs-time", "--", &request.id],
                    &cancel,
                )?
                .checked()?;
            let set = cleanly_snap::snapshot_set(&saved, &request.id)?;
            runner
                .run(
                    "/usr/bin/snap",
                    &["check-snapshot", &set, &request.id],
                    &cancel,
                )?
                .checked()?;
            println!(
                "Snap data snapshot {set} verified before removal. Restore with snap restore after reinstalling."
            );
            runner
                .run("/usr/bin/snap", &["remove", "--", &request.id], &cancel)?
                .checked()?;
        }
    }
    Ok(())
}
