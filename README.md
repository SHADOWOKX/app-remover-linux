# Cleanly

[![Safety and native build](https://github.com/SHADOWOKX/app-remover-linux/actions/workflows/ci.yml/badge.svg)](https://github.com/SHADOWOKX/app-remover-linux/actions/workflows/ci.yml)

A native Linux app uninstaller built with **Rust, GTK4 and libadwaita**. Cleanly shows how an app was installed, what belongs to it, and the exact removal plan before anything changes.

> **Public preview:** Cleanly 0.1.x is intentionally conservative. Authenticated real-package removal acceptance is still being completed in disposable VMs; see the [security review](docs/SECURITY-REVIEW.md) for the remaining release gates.

![Cleanly preview](assets/cleanly-preview.png)

## Features

- Review the complete removal plan before confirmation
- Shows app source, version, size and ownership evidence
- Supports **APT/dpkg, Flatpak, Snap and AppImage**
- Protects shared, unrelated and unproven files
- Native GNOME light/dark interface
- Quarantine and restore for eligible application data

## Install

### Ubuntu / Debian-based — amd64

Download and verify the current preview package:

```bash
curl -fLO https://github.com/SHADOWOKX/app-remover-linux/releases/download/v0.1.0/cleanly_0.1.0_amd64.deb
curl -fLO https://github.com/SHADOWOKX/app-remover-linux/releases/download/v0.1.0/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
sudo apt install ./cleanly_0.1.0_amd64.deb
```

Then open **Cleanly** from your applications menu, or run:

```bash
cleanly
```

The preview package is built by GitHub Actions on Ubuntu 24.04. Native desktop validation was performed on Ubuntu 26.04.1.

## Build from source

Requires Rust 1.93+.

```bash
git clone https://github.com/SHADOWOKX/app-remover-linux.git
cd app-remover-linux
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev
cargo build --workspace --release --locked
sudo make install
cleanly
```

## Security

Cleanly uses a fixed privileged helper for APT/Snap operations and keeps filesystem cleanup unprivileged. If ownership cannot be proven, the file is kept.

See the [Security Policy](SECURITY.md), [Implementation Security Review](docs/SECURITY-REVIEW.md), and [Validation Record](docs/VALIDATION.md).

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
