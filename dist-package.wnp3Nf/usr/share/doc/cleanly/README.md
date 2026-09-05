# Cleanly

A native GTK4/libadwaita application inspector and conservative Linux uninstaller.

**Select → inspect ownership → review an immutable plan → uninstall → verify.**

This repository contains a working **0.1 preview vertical slice**, not a claim that every production requirement is finished. Safety takes precedence over cleanup coverage. It discovers real installed applications; it is not a mockup. Package removals are never run by the test suite against the host.

## Build and run

Ubuntu 24.04 or later, with GTK ≥4.12, libadwaita ≥1.5, and Rust ≥1.93:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
cargo build --workspace --locked
cargo test --workspace --locked
cargo run -p cleanly-ui --bin cleanly
```

Install Rust 1.93 or newer if the distribution's compiler is older. Run the GUI as your normal user. `cargo run` supports inspection, native Flatpak operations, and AppImage quarantine. APT/Snap need the root-owned helper installed below; an uninstalled helper produces an explicit error and no removal.

```sh
cargo build --workspace --release --locked
sudo make install
cleanly
```

`make install` installs three binaries, desktop/AppStream metadata, an icon and a polkit policy under `/usr`. It does not launch the application as root. Authentication is per package operation, with no retained authorization policy. Flatpak uses its own system helper/polkit integration.

For a Debian package built on the target Ubuntu release:

```sh
make deb
sudo apt install ./dist/cleanly_0.1.0_amd64.deb
```

Do not assume a binary built on newer Ubuntu will run on older Ubuntu: rebuild on the target release. `make install PROFILE=debug DESTDIR="$PWD/dist/stage"` stages a debug build without administrator access. The helper path is fixed at `/usr/libexec/cleanly-helper`; do not relocate it independently from the policy and frontend.

## What works

| Source | Discovery / inspection | Removal behavior |
|---|---|---|
| APT/dpkg | Graphical desktop entries mapped to dpkg owners; version, architecture, installed-size estimate, package file inventory, shared/protected evidence | Single-package `dpkg --no-force-all --remove`; dry run, metadata/system protection, dependency checks, polkit helper and post-operation verification. No purge or autoremove. |
| Flatpak | Exact app ref, user/default-system scope, commit, origin, size, deployment, permissions and sandbox data | Exact-ref uninstall with `--no-related`; never runtime cleanup or `--delete-data`. Exclusive, validated sandbox data can be quarantined as one unit. |
| Snap | Graphical launchers, exact installed revision, publisher and local metadata; system types protected | Helper requires an explicit snapd snapshot and successful integrity check before exact-name removal. Never `--purge`. Restore snapd snapshots with `snap restore` after reinstalling. |
| Standalone | User-local and `/usr/local` launchers shown with unproven ownership | Inspection-only. Nothing is removed. |
| AppImage | Exact ELF/AppImage signature in three nonrecursive user directories; exact launcher references shown | Exact unshared, non-package-owned regular AppImage file moved to quarantine. Matching launchers/icons/settings remain untouched where exclusivity is unproven. |

The UI provides instantaneous in-memory name/ID search, source filtering, name/size/source sorting, a warm metadata cache, asynchronous inspection, a virtualized expandable file inspector, two removal modes, immutable confirmation, staged progress, honest partial results, local history, restore, keyboard shortcuts and System/Light/Dark appearance.

`Ctrl+F` focuses search; `Ctrl+R` refreshes discovery. Narrow windows collapse the application sidebar. Appearance persists locally. Unknown metadata stays unknown; installation dates and recursive home scans are not fabricated.

## Safety boundaries

- No application's `Exec=` line or AppImage is executed during discovery.
- No shell interpreter or command string is used by runtime process execution. Executables are fixed absolute paths; arguments are separate, validated values.
- Commands have cancellation, deadlines, bounded stdout/stderr, checked status and nonblocking pipe handling.
- Package-owned files are never manually removed. Shared dependencies are never recursively scheduled. Dpkg's dependency refusal is deliberately conservative, including desktop metapackage dependencies.
- Main GUI is unprivileged. The helper accepts only bounded, structured `RemoveApt` / `RemoveSnap` requests, clears its environment and independently rediscovers/protects the target.
- Selected cleanup requires Verified/Strong evidence and a matching inspected identity. Weak/Unknown entries are never deletable in this release.
- Cleanup is an atomic descriptor-relative quarantine move. There is no `remove_dir_all` in production code, no cross-device copy fallback, and no permanent cleanup command.
- Trees are bounded and checked in full. Unexpected symlinks, foreign ownership, hard links, special files, traversal, stale fingerprints and mount crossings cause preservation. Restore never overwrites an existing destination.
- Quarantine does **not** free space. Results report quarantined bytes separately; shared package-manager storage makes actual freed bytes unknown.

Bounded `events.toml` diagnostics contain only generated stage names, source types, counts, timestamps and success flags; no filenames, app names, command output or file contents. Private operation journals retain paths needed for recovery and are not privacy-scrubbed exports.

Private application state is in `~/.local/share/cleanly/` (0700), with append-only operation plans/results and `quarantine/<operation-id>/record.toml` plus `payload`. Files are 0600. Quarantine moves require the same filesystem. Plans are journaled before mutation. An interruption may leave an intent record without a result; inspect the journal and underlying package-manager state before retrying. Package operations cannot generally be rolled back by restoring personal data.

The threat model and residual limitations are in [docs/SECURITY-REVIEW.md](docs/SECURITY-REVIEW.md). This preview has not received an independent security certification.

## Deliberately unfinished

- General standalone/manual installation removal (discovery is available) and arbitrary XDG leftover correlation: ownership is not yet proven, so these are kept.
- Automatic quarantine expiry and permanent deletion: not enabled. Retention is until manually managed.
- Separate Flatpak cache/config/state selection: the exclusive sandbox is treated as a single data unit.
- Custom AppImage search roots and XDG root overrides: discovery is currently limited to `~/Applications`, `~/.local/bin`, `~/Downloads`; no recursive scan.
- Autoremove, purge, custom Flatpak system installations, reliable install-date sorting, measured reclaimed filesystem blocks, runtime-level rollback, log-sharing export UI and advanced storage charts.
- VM coverage of authenticated real APT/Snap/Flatpak uninstall transactions and adversarial concurrent package changes. Only isolated fixture cleanup is executed by automated tests.

These limitations are visible in the product; they are not hidden behind enabled but unsafe controls.

## Architecture

- `cleanly-core`: GTK-independent models, ownership evidence, protection-aware immutable plan construction, backend trait and cancellation.
- `cleanly-platform`: bounded subprocesses, restricted desktop metadata parser, fingerprinted descriptor-relative quarantine and restore.
- `cleanly-apt`, `cleanly-flatpak`, `cleanly-snap`, `cleanly-appimage`: isolated discovery/inspection adapters.
- `cleanly-service`: parallel backend isolation, lazy inspection, plan preparation, revalidation, execution, verification, history and cache; `cleanly-helper` and read-only `cleanly-inspect` binaries.
- `cleanly-ui`: native GTK4/libadwaita frontend. No discovery logic depends on GTK. Five source workers bound discovery concurrency; only the current inspection is retained/cancellable. File inspection is bounded at 100,000 entries, depth 64, and ten seconds per tree.

## Tests and diagnostics

```sh
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p cleanly-service --bin cleanly-inspect
cargo run -p cleanly-service --bin cleanly-inspect -- inspect 'Flatpak:system:app/org.mozilla.firefox/x86_64/stable'
```

Tests use mocked command output and private temporary fixtures, including real quarantine/restore round trips. The read-only CLI is useful for checking discovery on another distribution. Source failures are isolated; hover the source status in the UI for exact diagnostics.

Optional GNOME smoke test (read-only, saves a screenshot to `/tmp/cleanly-smoke.png`, exits automatically):

```sh
CLEANLY_SMOKE_TEST=1 GSK_RENDERER=cairo target/debug/cleanly
```

Backend semantics follow the [dpkg manual](https://manpages.ubuntu.com/manpages/resolute/man1/dpkg.1.html), [Flatpak command reference](https://docs.flatpak.org/en/latest/flatpak-command-reference.html), and [Snap snapshot documentation](https://snapcraft.io/docs/how-to-guides/manage-snaps/create-data-snapshots/).

GPL-3.0-or-later. See [LICENSE](LICENSE).
