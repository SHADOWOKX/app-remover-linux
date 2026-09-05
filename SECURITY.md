# Security Policy

Cleanly handles destructive filesystem operations and includes a small privileged helper, so security reports are taken seriously.

## Supported versions

| Version | Status |
| --- | --- |
| 0.1.x | Public preview — security fixes accepted |

## Reporting a vulnerability

Please **do not publish exploit details, sensitive paths, or proof-of-concept payloads in a public issue**.

1. If GitHub shows **Report a vulnerability** on this repository's Security tab, use that private channel.
2. If private reporting is unavailable, open a public issue titled **Security contact request** with only the affected version and a way to identify your GitHub account. Do not include technical exploit details; a private exchange can then be arranged.

A useful report includes the affected version/commit, Linux distribution, installation method, required privileges, minimal reproduction steps, security impact, and any relevant logs with personal paths or tokens removed.

For normal crashes, UI problems, feature requests, or non-security bugs, use the public issue tracker.

## Security design notes

The implementation threat model, privileged-helper boundary, tested attack classes, known limitations, and remaining release gates are documented in [docs/SECURITY-REVIEW.md](docs/SECURITY-REVIEW.md). The current validation record is in [docs/VALIDATION.md](docs/VALIDATION.md).
