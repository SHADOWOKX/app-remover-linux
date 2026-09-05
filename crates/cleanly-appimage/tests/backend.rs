use cleanly_appimage::identify;
use cleanly_platform::files::operation_id;
#[test]
fn name_alone_is_not_appimage() {
    let p = std::env::temp_dir().join(format!("{}.AppImage", operation_id()));
    std::fs::write(&p, b"not an AppImage").unwrap();
    assert!(identify(&p).is_err());
    std::fs::remove_file(p).unwrap();
}
#[test]
fn exact_header_identifies_without_execution() {
    let p = std::env::temp_dir().join(operation_id());
    std::fs::write(&p, b"\x7fELF\x02\x01\x01\x00AI\x02rest").unwrap();
    assert!(identify(&p).is_ok());
    std::fs::remove_file(p).unwrap();
}
