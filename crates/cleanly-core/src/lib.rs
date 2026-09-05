//! GTK-independent domain model. Plans are constructed through validation, never deserialized.
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn check(&self) -> Result<()> {
        if self.0.load(Ordering::Relaxed) {
            Err("Operation cancelled".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Apt,
    Flatpak,
    Snap,
    AppImage,
    Manual,
}
impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Self::Apt => "APT",
            Self::Flatpak => "Flatpak",
            Self::Snap => "Snap",
            Self::AppImage => "AppImage",
            Self::Manual => "Standalone",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledApp {
    pub id: String,
    pub name: String,
    pub version: String,
    pub backend: Backend,
    pub scope: String,
    pub architecture: String,
    pub publisher: String,
    pub location: PathBuf,
    pub icon: String,
    pub size: Option<u64>,
    pub protection: Option<String>,
}
impl InstalledApp {
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.backend.label(), self.scope, self.id)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Verified,
    Strong,
    Weak,
    Unknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileCategory {
    Application,
    Configuration,
    Cache,
    Data,
    State,
    Integration,
    Protected,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OwnershipEvidence {
    PackageDatabase {
        package: String,
        owners: Vec<String>,
    },
    FlatpakSandbox {
        app_id: String,
        exclusive: bool,
    },
    AppImageIdentity {
        executable: PathBuf,
    },
    ExactLauncher {
        executable: PathBuf,
    },
    Hint {
        reason: String,
    },
    Protected {
        reason: String,
    },
}
impl OwnershipEvidence {
    pub fn description(&self) -> String {
        match self {
            Self::PackageDatabase { package, owners } => format!(
                "dpkg database: {package}. Registered owners: {}. Removed only by dpkg.",
                owners.join(", ")
            ),
            Self::FlatpakSandbox { app_id, exclusive } => format!(
                "Flatpak sandbox namespace for {app_id}. Exclusive installed identity: {exclusive}."
            ),
            Self::AppImageIdentity { executable } => format!(
                "Exact regular file with ELF/AppImage signature: {}",
                executable.display()
            ),
            Self::ExactLauncher { executable } => format!(
                "Launcher references exact executable {}. Kept unless exclusive ownership can be proven.",
                executable.display()
            ),
            Self::Hint { reason } | Self::Protected { reason } => reason.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub device: u64,
    pub inode: u64,
    pub mode: u32,
    pub size: u64,
    pub mtime: i64,
    pub mtime_ns: i64,
    pub ctime: i64,
    pub ctime_ns: i64,
    pub links: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovalCandidate {
    pub path: PathBuf,
    pub ownership: OwnershipEvidence,
    pub confidence: Confidence,
    pub category: FileCategory,
    pub size: Option<u64>,
    pub selected_by_default: bool,
    pub fingerprint: Option<Fingerprint>,
}
impl RemovalCandidate {
    pub fn cleanup_allowed(&self) -> bool {
        if !matches!(self.confidence, Confidence::Verified | Confidence::Strong)
            || self.category == FileCategory::Protected
        {
            return false;
        }
        matches!(
            &self.ownership,
            OwnershipEvidence::FlatpakSandbox {
                exclusive: true,
                ..
            } | OwnershipEvidence::AppImageIdentity { .. }
        ) && self.fingerprint.is_some()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppManifest {
    pub app: InstalledApp,
    pub files: Vec<RemovalCandidate>,
    pub notes: Vec<String>,
    pub backend_token: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemovalMode {
    Uninstall,
    Complete,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RemovalPlan {
    manifest: AppManifest,
    mode: RemovalMode,
    selected: Vec<usize>,
}
impl RemovalPlan {
    pub fn build(manifest: AppManifest, mode: RemovalMode, selected: Vec<usize>) -> Result<Self> {
        if let Some(reason) = &manifest.app.protection {
            return Err(reason.clone());
        }
        if manifest.app.backend == Backend::Manual {
            return Err("Manual ownership is unproven; removal disabled".into());
        }
        for &i in &selected {
            let file = manifest.files.get(i).ok_or("Invalid selection")?;
            if !file.cleanup_allowed() {
                return Err(
                    "Unproven, shared or package-owned file cannot be manually removed".into(),
                );
            }
            validate_absolute(&file.path)?;
        }
        let mut unique = selected.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != selected.len() {
            return Err("Duplicate selection".into());
        }
        if manifest.app.backend == Backend::AppImage
            && !selected
                .iter()
                .any(|&i| manifest.files[i].category == FileCategory::Application)
        {
            return Err("Select the exact AppImage file".into());
        }
        Ok(Self {
            manifest,
            mode,
            selected,
        })
    }
    pub fn manifest(&self) -> &AppManifest {
        &self.manifest
    }
    pub fn selected(&self) -> &[usize] {
        &self.selected
    }
    pub fn mode(&self) -> RemovalMode {
        self.mode
    }
}
pub trait PackageBackend: Send + Sync {
    fn kind(&self) -> Backend;
    fn discover(&self, cancel: &Cancellation) -> Result<Vec<InstalledApp>>;
    fn inspect(&self, app: &InstalledApp, cancel: &Cancellation) -> Result<AppManifest>;
}
pub fn validate_absolute(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err("Path must be absolute without traversal".into());
    }
    // Components normalizes embedded dots, so check raw components as well.
    if path
        .as_os_str()
        .as_encoded_bytes()
        .split(|&b| b == b'/')
        .any(|s| s == b"." || s == b"..")
    {
        return Err("Path traversal rejected".into());
    }
    Ok(())
}
pub fn valid_package(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && (s.as_bytes()[0].is_ascii_lowercase() || s.as_bytes()[0].is_ascii_digit())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-:".contains(&b))
}
pub fn valid_snap(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}
pub fn valid_app_id(s: &str) -> bool {
    s.len() <= 255
        && s.split('.').count() >= 3
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}
pub fn format_size(bytes: Option<u64>) -> String {
    match bytes {
        None => "Unknown".into(),
        Some(n) if n >= 1 << 30 => format!("{:.1} GiB", n as f64 / (1u64 << 30) as f64),
        Some(n) if n >= 1 << 20 => format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64),
        Some(n) if n >= 1 << 10 => format!("{:.1} KiB", n as f64 / 1024.),
        Some(n) => format!("{n} B"),
    }
}
