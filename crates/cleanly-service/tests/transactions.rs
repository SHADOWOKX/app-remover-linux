use cleanly_core::*;
use cleanly_platform::{Output, Runner, files};
use cleanly_service::{Service, privileged::Request};
use std::{fs, path::PathBuf, sync::Arc};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("cleanly-execution-{}", files::operation_id()));
        fs::create_dir_all(path.join("Applications")).unwrap();
        Self(path)
    }
    fn image(&self) -> PathBuf {
        let path = self.0.join("Applications/Example.AppImage");
        fs::write(&path, b"\x7fELF\x02\x01\x01\x00AI\x02test payload").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct NoOwner;
impl Runner for NoOwner {
    fn run(&self, program: &str, args: &[&str], _: &Cancellation) -> Result<Output> {
        assert_eq!(program, "/usr/bin/dpkg-query");
        assert_eq!(&args[..2], &["-S", "--"]);
        Ok(Output {
            success: false,
            stdout: String::new(),
            stderr: format!("dpkg-query: no path found matching pattern {}", args[2]),
        })
    }
}
fn app(path: PathBuf) -> InstalledApp {
    InstalledApp {
        id: path.to_string_lossy().into(),
        name: "Example".into(),
        version: "Unknown".into(),
        backend: Backend::AppImage,
        scope: "user".into(),
        architecture: "Unknown".into(),
        publisher: "Unknown".into(),
        location: path,
        icon: String::new(),
        size: None,
        protection: None,
    }
}
#[test]
fn complete_appimage_vertical_slice_and_restore() {
    let f = Fixture::new();
    let path = f.image();
    let service = Service {
        home: f.0.clone(),
        runner: Arc::new(NoOwner),
        operations: Arc::new(NoOwner),
    };
    let cancel = Cancellation::default();
    let manifest = service.inspect(&app(path.clone()), &cancel).unwrap();
    let plan = service
        .prepare(manifest, RemovalMode::Uninstall, vec![0], &cancel)
        .unwrap();
    let result = service.execute(plan, |_| {}, &cancel).unwrap();
    assert!(result.package_removed);
    assert!(result.errors.is_empty());
    assert!(!path.exists());
    assert_eq!(service.history().unwrap().len(), 1);
    assert_eq!(result.quarantined.len(), 1);
    files::restore(&f.0, &result.quarantined[0]).unwrap();
    assert!(path.exists());
}
#[test]
fn stale_execution_keeps_replacement() {
    let f = Fixture::new();
    let path = f.image();
    let service = Service {
        home: f.0.clone(),
        runner: Arc::new(NoOwner),
        operations: Arc::new(NoOwner),
    };
    let c = Cancellation::default();
    let manifest = service.inspect(&app(path.clone()), &c).unwrap();
    let plan = service
        .prepare(manifest, RemovalMode::Uninstall, vec![0], &c)
        .unwrap();
    fs::rename(&path, path.with_extension("old")).unwrap();
    fs::write(&path, b"unrelated personal file").unwrap();
    assert!(service.execute(plan, |_| {}, &c).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"unrelated personal file");
}
#[test]
fn helper_request_rejects_injection_and_extra_fields() {
    for text in [
        "action = 'RemoveApt'\nid = '--all'\nversion = '1'\ntoken = '1'",
        "action = 'RemoveApt'\nid = 'example'\nversion = '1'\ntoken = '1'\ncommand = 'rm -rf /'",
        "action = 'ExecuteShell'\nid = 'example'\nversion = '1'\ntoken = '1'",
    ] {
        assert!(Request::parse(text).is_err());
    }
}
#[test]
fn helper_request_only_fixed_actions() {
    assert!(Request::parse("action = 'RemoveApt'\nid = 'example:amd64'\nversion = '1:2.0-1'\ntoken = 'example:amd64:1:2.0-1'").is_ok());
}
use std::sync::atomic::{AtomicBool, Ordering};
struct FlatpakMock {
    removed: AtomicBool,
    fail: bool,
}
impl Runner for FlatpakMock {
    fn run(&self, program: &str, args: &[&str], _: &Cancellation) -> Result<Output> {
        assert_eq!(program, "/usr/bin/flatpak");
        let text = match args[0] {
            "list" => {
                if args[1] == "--user" && !self.removed.load(Ordering::SeqCst) {
                    "org.example.App/x86_64/stable\tExample\t1\tflathub\t4 kB\tuser\n"
                } else {
                    ""
                }
            }
            "info" => match args[2] {
                "--show-commit" => "commit-1",
                "--show-location" => "/tmp/flatpak-deployment",
                "--show-size" => "4096",
                "--show-metadata" => "[Application]\nname=org.example.App",
                "--show-permissions" => "",
                _ => panic!("Unexpected query"),
            },
            "uninstall" => {
                assert_eq!(
                    args,
                    &[
                        "uninstall",
                        "--user",
                        "--app",
                        "--no-related",
                        "--noninteractive",
                        "--",
                        "app/org.example.App/x86_64/stable"
                    ]
                );
                if self.fail {
                    return Ok(Output {
                        success: false,
                        stdout: String::new(),
                        stderr: "Mock permission denied".into(),
                    });
                }
                self.removed.store(true, Ordering::SeqCst);
                ""
            }
            _ => panic!("Unexpected command"),
        };
        Ok(Output {
            success: true,
            stdout: text.into(),
            stderr: String::new(),
        })
    }
}
#[test]
fn mocked_flatpak_uninstall_verifies_and_quarantines_only_owned_data() {
    let f = Fixture::new();
    let data = f.0.join(".var/app/org.example.App");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("settings"), "private").unwrap();
    let other = f.0.join(".var/app/org.other.App");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("keep"), "unrelated").unwrap();
    let mock = Arc::new(FlatpakMock {
        removed: AtomicBool::new(false),
        fail: false,
    });
    let service = Service {
        home: f.0.clone(),
        runner: mock.clone(),
        operations: mock,
    };
    let c = Cancellation::default();
    let app = service
        .backend(Backend::Flatpak)
        .unwrap()
        .discover(&c)
        .unwrap()
        .remove(0);
    let manifest = service.inspect(&app, &c).unwrap();
    let selection = manifest
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.cleanup_allowed())
        .map(|(i, _)| i)
        .collect();
    let plan = service
        .prepare(manifest, RemovalMode::Complete, selection, &c)
        .unwrap();
    let result = service.execute(plan, |_| {}, &c).unwrap();
    assert!(result.package_removed);
    assert!(result.errors.is_empty());
    assert!(!data.exists());
    assert!(other.join("keep").exists());
    assert_eq!(result.quarantined_bytes, 7);
}
#[test]
fn failed_package_removal_never_cleans_data() {
    let f = Fixture::new();
    let data = f.0.join(".var/app/org.example.App");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("settings"), "keep").unwrap();
    let mock = Arc::new(FlatpakMock {
        removed: AtomicBool::new(false),
        fail: true,
    });
    let service = Service {
        home: f.0.clone(),
        runner: mock.clone(),
        operations: mock,
    };
    let c = Cancellation::default();
    let app = service
        .backend(Backend::Flatpak)
        .unwrap()
        .discover(&c)
        .unwrap()
        .remove(0);
    let manifest = service.inspect(&app, &c).unwrap();
    let selection = manifest
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| f.cleanup_allowed())
        .map(|(i, _)| i)
        .collect();
    let plan = service
        .prepare(manifest, RemovalMode::Complete, selection, &c)
        .unwrap();
    let result = service.execute(plan, |_| {}, &c).unwrap();
    assert!(!result.package_removed);
    assert!(!result.errors.is_empty());
    assert!(data.join("settings").exists());
    assert_eq!(result.quarantined_bytes, 0);
}
