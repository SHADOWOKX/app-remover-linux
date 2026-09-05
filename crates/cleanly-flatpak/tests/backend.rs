use cleanly_flatpak::*;
#[test]
fn exact_ref_and_scope() {
    let apps = parse_list(
        "app/org.example.App/x86_64/stable\tExample\t1\tflathub\t10 MB\tuser\n",
        "user",
    )
    .unwrap();
    assert_eq!(apps[0].scope, "user");
    assert_eq!(apps[0].id, "app/org.example.App/x86_64/stable");
}
#[test]
fn reject_runtime_partial_and_options() {
    for id in [
        "org.example.App",
        "runtime/org.example.App/x86_64/stable",
        "app/org.example.App/x86_64/--all",
        "--all",
    ] {
        assert!(!valid_ref(id));
    }
}
#[test]
fn malformed_row_fails() {
    assert!(parse_list("org.example.App Example", "user").is_err());
}
#[test]
fn real_flatpak_list_omits_app_prefix() {
    let app = parse_list(
        "org.gnome.Calendar/x86_64/stable\tCalendar\t50.0\tflathub\t15.8 MB\tsystem\n",
        "system",
    )
    .unwrap()
    .remove(0);
    assert_eq!(app.id, "app/org.gnome.Calendar/x86_64/stable");
    assert_eq!(app.size, Some(15_800_000));
}
