# Cleanly 0.1 implementation security review

Scope: all application runtime crates, helper, packaging policy, and fixture tests. This is a source-backed implementation review by the implementing agent, not an independent security assessment or a production certification. Real installed applications were inspected; no real package was uninstalled for testing.

## Trust boundaries

Untrusted inputs: desktop-entry text, display names/icons, user filenames, cached records, selected identities and structured helper requests. The main application is not trusted by the privileged helper. Root-owned package databases, installed package maintainer scripts, snapd, Flatpak/OSTree and the Linux kernel are trusted system components. Administrator authorization permits removing an exact installed application, not executing arbitrary GUI-provided commands.

Filesystem cleanup runs only with the user's privileges. Another UID cannot modify Cleanly's private 0700 state. A malicious process with the **same UID** can modify that user's files and quarantine records: Linux does not provide a general atomic “rename/unlink only this inode” operation. Descriptor anchoring, full-tree checks, no-replace renames, post-move checks and rollback detect common substitution races, but do not establish isolation from a fully hostile same-UID process. This is one reason permanent deletion and automatic expiry are disabled.

## Audited attack classes

| Class | Control / validation |
|---|---|
| Command injection | Fixed executable paths; argument arrays; package/ref validation; no runtime shell interpreter; tests preserve shell syntax literally and reject helper command fields. |
| Path traversal | Absolute-path validation rejects raw dot/dot-dot and NUL components; only exact sandbox roots or AppImage children in bounded search roots are accepted. |
| Symlink attacks | Directory components opened with `openat(O_NOFOLLOW|O_DIRECTORY)`; tree roots/children reject symlinks; restore opens and anchors original parent; no symlink traversal during cleanup. |
| TOCTOU / stale files | Device, inode, type/mode, size, link count, mtime and ctime captured; Linux statx mount IDs reject same-device bind-mount crossings; whole-tree plan snapshot rechecked; atomic rename, post-move verification and no-overwrite rollback. Package-manager version/ref revalidated immediately before command. |
| Privilege escalation | GUI refuses root; root-owned fixed helper and ancestors checked; polkit active-session `auth_admin`; two deny-unknown-fields request actions; helper independently discovers target, checks protection and clears environment. |
| Recursive deletion | No production recursive deletion. Full bounded tree validation precedes quarantine. Same-filesystem rename only, no copy/delete fallback. |
| Package ownership confusion | Dpkg file inventory plus exact owner records. Multiple/unknown owners remain protected. AppImage cleanup rechecks dpkg registration. Package paths cannot be selected for manual cleanup. |
| Shared dependencies | Single explicit dpkg target, no force flags or dependency solver cleanup; dry-run dependency refusal. Flatpak exact app ref with `--no-related`, no runtime/unused operations. |
| Malicious desktop metadata | Bounded visible-application parser; duplicate keys rejected; Exec never executed; restricted exact executable extraction is review-only; UI markup disabled for untrusted text. |
| Malicious AppImage metadata | Only ELF/AppImage header and filesystem identity inspected. No mount, execution, extraction or embedded metadata process. Signature is classification, not publisher authenticity. |
| Untrusted filenames | OS paths kept as paths; no shell interpolation; non-UTF8/ambiguous package-query patterns fail closed for AppImage removal. |
| Process races / hangs | Nonblocking bounded pipes, process-group deadline/cancellation, status check; failed/ambiguous removal prevents data cleanup. Independent discovery source results. |
| Data loss / misleading results | Snap requires explicit checked snapshot; application data defaults preserved; unknown/shared/protected files never manually removed; quarantine bytes never called freed bytes; verification determines success. |

## Issues found and corrected during implementation

- Flatpak branch/ref validation accepted option-shaped components. Validator tightened and a regression test added.
- Debian numeric package names caused whole-backend discovery failure. The Debian package grammar and regression fixture were corrected.
- Flatpak's real list schema lacked the assumed `app/` prefix. Normalize only app-list records into exact refs, with regression coverage.
- GTK expander title treated a literal ampersand as markup. Markup disabled; review text uses plain labels.
- Snap automatic snapshots can be administratively disabled. The helper now requires an explicit snapshot and integrity check before removal; system Snap types are protected using installed metadata.
- Desktop/AppImage metadata readers could block on a substituted FIFO after an initial stat. Descriptor-relative `O_NOFOLLOW|O_NONBLOCK` regular-file readers and a regression test close that path.
- A delayed discovery batch could outlive a refresh. Separate scan and inspection generations reject stale batches.

## Residual release gates

1. Package-manager execution is serialized by the underlying manager, but its transaction lock is not held across Cleanly's preview and invocation. An administrator can update the same identity between the version check and operation. The target cannot widen to another identifier, yet the reviewed version may be stale. VM tests and a stronger transaction interface are required before calling this production-ready.
2. Package maintainer scripts and Snap hooks are trusted and may have effects beyond the static package file inventory. Cleanly does not sandbox or rewrite them.
3. A fully hostile same-UID writer can race filesystem moves or alter quarantine metadata. No permanent deletion is enabled. Interrupted moves retain a recovery record; rollback never overwrites new data.
4. Tree scans are conservative: nested symlinks/hard links cause the entire cleanup tree to be kept, which commonly preserves otherwise removable Flatpak data.
5. Only default user/system Flatpak installations are enumerated. Custom installation scopes are outside this preview; sandbox cleanup is disabled when additional installation files or environment overrides exist.
6. Quarantine/history can contain diagnostic paths. There is no log export/redaction UI yet; do not publish raw journals as privacy-scrubbed reports.
7. Automatic retention, arbitrary XDG cleanup, broad standalone removal, measured freed space and aggressive integrations are deliberately unavailable.

No existing application's package or personal data was removed on the development host. Authenticated backend mutations require subsequent isolated-VM acceptance tests before a production release.
