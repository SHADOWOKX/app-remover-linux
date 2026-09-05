use cleanly_apt::*;
use cleanly_core::*;
use cleanly_platform::{Output, Runner};
use std::sync::Arc;
struct Mock;
impl Runner for Mock {
    fn run(&self, program: &str, args: &[&str], _: &Cancellation) -> Result<Output> {
        assert_eq!(program, "/usr/bin/dpkg-query");
        assert_eq!(&args[args.len() - 2..], &["--", "example"]);
        Ok(Output {
            stdout: "example\t1.2\tamd64\t100\tno\tno\toptional\tutils\tinstalled\n".into(),
            stderr: String::new(),
            success: true,
        })
    }
}
#[test]
fn mock_package_metadata() {
    let app = Apt {
        runner: Arc::new(Mock),
    }
    .package("example", &Cancellation::default())
    .unwrap();
    assert_eq!(app.version, "1.2");
    assert_eq!(app.size, Some(102400));
    assert!(app.protection.is_none());
}
#[test]
fn malformed_metadata_fails_closed() {
    assert!(parse_packages("some human output").is_err());
}
#[test]
fn multiple_owners_remain_distinct() {
    let owners =
        parse_owners("example:amd64, other: /usr/share/shared\nexample: /usr/bin/example\n");
    assert_eq!(owners[std::path::Path::new("/usr/share/shared")].len(), 2);
}
#[test]
fn metadata_and_category_protect_system() {
    for (id, e, p, priority, section) in [
        ("unlisted", "yes", "no", "optional", "utils"),
        ("unlisted", "no", "yes", "optional", "utils"),
        ("unlisted", "no", "no", "required", "utils"),
        ("linux-image-example", "no", "no", "optional", "kernel"),
        ("gnome-shell", "no", "no", "optional", "gnome"),
    ] {
        assert!(protected(id, e, p, priority, section).is_some());
    }
}
#[test]
fn numeric_package_prefix_is_valid_debian_metadata() {
    assert!(
        parse_packages("3cpio\t1\tamd64\t10\tno\tno\toptional\tutils\tinstalled\n")
            .unwrap()
            .contains_key("3cpio")
    );
}
