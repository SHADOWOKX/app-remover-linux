use cleanly_core::*;
use std::path::PathBuf;
fn app() -> InstalledApp {
    InstalledApp {
        id: "org.example.App".into(),
        name: "Example".into(),
        version: "1".into(),
        backend: Backend::Flatpak,
        scope: "user".into(),
        architecture: "Unknown".into(),
        publisher: "Unknown".into(),
        location: "/tmp/app".into(),
        icon: String::new(),
        size: None,
        protection: None,
    }
}
fn candidate() -> RemovalCandidate {
    RemovalCandidate {
        path: "/home/user/.var/app/org.example.App".into(),
        ownership: OwnershipEvidence::FlatpakSandbox {
            app_id: "org.example.App".into(),
            exclusive: true,
        },
        confidence: Confidence::Strong,
        category: FileCategory::Data,
        size: Some(1),
        selected_by_default: true,
        fingerprint: Some(Fingerprint {
            device: 1,
            inode: 1,
            mode: 0o40700,
            size: 1,
            mtime: 1,
            mtime_ns: 0,
            ctime: 1,
            ctime_ns: 0,
            links: 1,
        }),
    }
}
fn plan(file: RemovalCandidate) -> Result<RemovalPlan> {
    RemovalPlan::build(
        AppManifest {
            app: app(),
            files: vec![file],
            notes: vec![],
            backend_token: "1".into(),
        },
        RemovalMode::Complete,
        vec![0],
    )
}
#[test]
fn verified_sandbox_can_be_selected() {
    assert!(plan(candidate()).is_ok());
}
#[test]
fn weak_and_unknown_never_cleaned() {
    for confidence in [Confidence::Weak, Confidence::Unknown] {
        let mut c = candidate();
        c.confidence = confidence;
        assert!(plan(c).is_err());
    }
}
#[test]
fn shared_sandbox_never_cleaned() {
    let mut c = candidate();
    c.ownership = OwnershipEvidence::FlatpakSandbox {
        app_id: "org.example.App".into(),
        exclusive: false,
    };
    assert!(plan(c).is_err());
}
#[test]
fn package_files_never_manually_cleaned() {
    for owners in [
        vec!["example".into()],
        vec!["example".into(), "other".into()],
    ] {
        let mut c = candidate();
        c.ownership = OwnershipEvidence::PackageDatabase {
            package: "example".into(),
            owners,
        };
        assert!(plan(c).is_err());
    }
}
#[test]
fn resemblance_is_not_evidence() {
    let mut c = candidate();
    c.ownership = OwnershipEvidence::Hint {
        reason: "Same name".into(),
    };
    assert!(plan(c).is_err());
}
#[test]
fn protected_cannot_be_selected() {
    let mut c = candidate();
    c.category = FileCategory::Protected;
    assert!(plan(c).is_err());
}
#[test]
fn missing_fingerprint_rejected() {
    let mut c = candidate();
    c.fingerprint = None;
    assert!(plan(c).is_err());
}
#[test]
fn traversal_rejected() {
    for p in [
        "/home/user/../other",
        "/home/./user",
        "relative/path",
        "/tmp/a\0b",
    ] {
        assert!(validate_absolute(&PathBuf::from(p)).is_err(), "{p}");
    }
}
#[test]
fn identifiers_cannot_be_options_or_shell() {
    for id in [
        "--all",
        "foo;rm -rf /",
        "$(touch /tmp/bad)",
        "foo\nbar",
        "../x",
        "",
    ] {
        assert!(!valid_package(id));
        assert!(!valid_snap(id));
    }
}
#[test]
fn app_identifiers_are_strict() {
    assert!(valid_app_id("org.gnome.Calculator"));
    for id in ["org..App", "../App", "--delete-data", "org.example/evil"] {
        assert!(!valid_app_id(id));
    }
}
#[test]
fn system_protection_blocks_plan() {
    let mut app = app();
    app.protection = Some("Essential".into());
    assert!(
        RemovalPlan::build(
            AppManifest {
                app,
                files: vec![],
                notes: vec![],
                backend_token: String::new()
            },
            RemovalMode::Uninstall,
            vec![]
        )
        .is_err()
    );
}
#[test]
fn duplicates_and_out_of_bounds_rejected() {
    let m = AppManifest {
        app: app(),
        files: vec![candidate()],
        notes: vec![],
        backend_token: String::new(),
    };
    assert!(RemovalPlan::build(m.clone(), RemovalMode::Complete, vec![0, 0]).is_err());
    assert!(RemovalPlan::build(m, RemovalMode::Complete, vec![10]).is_err());
}
