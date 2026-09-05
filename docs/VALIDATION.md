# Cleanly 0.1 validation record

Validated September 5, 2026 on Ubuntu 26.04.1 GNOME, Rust 1.93.1, GTK 4.22.4 and libadwaita 1.9.1.

- `cargo test --workspace --locked --offline`: **50 passed, 0 failed**. Temporary fixture cleanup only; no host package removals.
- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets --locked --offline -- -D warnings`: passed without warnings.
- `cargo build --workspace --release --locked --offline`: passed, optimized binaries generated.
- Native GNOME launch: **58 real applications discovered**, including APT, Flatpak, Snap, AppImage and protected standalone launchers. A sandbox that cannot access snapd reports a source timeout independently.
- Real APT inspection: Brave's package inventory produced **272 file entries**, with exact dpkg ownership and a successful non-destructive dependency check.
- Real Flatpak inspection: Firefox's exact default-system ref, commit, size, deployment and sandbox evidence loaded. Its sandbox was kept protected because full cleanup validation did not pass.
- Native UI screenshots reviewed for normal, light, narrow, file-inspector and confirmation views. Final normal-view runtime log contained **no GTK warnings**. Smoke mode never confirms or executes removal.
- Desktop file validation: passed. Policy, icon and AppStream XML parse successfully.
- Debian package built and inspected. `tests/package-smoke.py` verifies package metadata and checksum, root ownership, 0755 directories/executables, no setuid bits, the fixed polkit helper boundary and byte-for-byte equality with the current release binaries.

The local formatter/linter were downloaded as Ubuntu packages into `/tmp` and extracted there; no development packages were installed globally. The application helper was packaged but **not installed into the host system**.

Repository CI additionally builds/tests on Ubuntu 24.04, validates AppStream metadata, creates the Debian package, runs the package acceptance test, and rejects release binaries containing the CI build-home path. Release publication is gated on a successful main-branch CI run.

## Remaining validation limitations

- Authenticated real APT, Flatpak and Snap removals have not been run against the developer's machine. Use disposable VMs for those acceptance tests; their backend output and failure cases are mocked in the test suite.
- Native desktop/runtime acceptance was performed on Ubuntu 26.04.1. CI compilation and packaging on Ubuntu 24.04 does not replace a clean 24.04 installation/runtime smoke test.
- AppStream metadata now includes the public project homepage and bug tracker. Package metadata uses the GitHub noreply maintainer address rather than a personal email address.
- `dpkg-shlibdeps` may emit host-layout warnings around merged-/usr loader paths. Imported-symbol dependency requirements are generated and the archive permissions/binaries are independently checked.
- Startup/search latency targets have not been benchmarked across representative hardware.
- This is a reviewed and tested vertical slice, **not a production security certification**. See SECURITY-REVIEW.md for transaction-race and same-UID threat limits.
