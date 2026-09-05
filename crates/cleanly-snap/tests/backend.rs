use cleanly_snap::*;
#[test]
fn snap_metadata() {
    let apps=parse_list("Name Version Rev Tracking Publisher Notes\nexample 1 42 latest/stable publisher classic\ncore22 22 1 latest/stable canonical base\n").unwrap();
    assert_eq!(apps[0].location.to_str(), Some("/snap/example/42"));
    assert!(apps[1].protection.is_some());
}
#[test]
fn snap_malformed_rejected() {
    assert!(parse_list("unexpected human output").is_err());
}
#[test]
fn snapshot_set_is_tied_to_exact_snap() {
    assert_eq!(
        snapshot_set(
            "Set Snap Age Version Rev Size Notes\n12 example 2026-09-05T12:00:00Z 1 42 100B -",
            "example"
        )
        .unwrap(),
        "12"
    );
    assert!(
        snapshot_set(
            "Set Snap Age Version Rev Size Notes\n12 another 1s 1 42 100B -",
            "example"
        )
        .is_err()
    );
}
#[test]
fn malicious_revision_is_rejected() {
    assert!(!valid_revision("../../etc"));
    assert!(valid_revision("x1"));
    assert!(
        parse_list(
            "Name Version Rev Tracking Publisher Notes\nexample 1 ../../etc stable publisher -"
        )
        .is_err()
    );
}
