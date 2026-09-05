#!/bin/sh
set -eu

profile=${1:-release}
case "$profile" in
  release|debug) ;;
  *) echo 'Expected release or debug' >&2; exit 1 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

version=$(awk -F '"' '/^version = "/ { print $2; exit }' Cargo.toml)
if [ -z "$version" ]; then
  echo 'Unable to determine package version from Cargo.toml' >&2
  exit 1
fi

maintainer='SHADOWOKX'
homepage='https://github.com/SHADOWOKX/app-remover-linux'

# Release artifacts must not embed the developer's local source/home path.
remap_flags="--remap-path-prefix=$root=/usr/src/cleanly"
if [ -n "${HOME:-}" ]; then
  remap_flags="$remap_flags --remap-path-prefix=$HOME=/usr/src/build-home"
fi

# Rebuild package binaries from scratch with privacy-safe source paths.
rm -rf "target/$profile"
CARGO_INCREMENTAL=0 RUSTFLAGS="$remap_flags ${RUSTFLAGS:-}" \
  make all "PROFILE=$profile"

for binary in \
  "target/$profile/cleanly" \
  "target/$profile/cleanly-inspect" \
  "target/$profile/cleanly-helper"
do
  if LC_ALL=C grep -aF "$root" "$binary" >/dev/null 2>&1; then
    echo "Refusing to package $binary: build path is embedded" >&2
    exit 1
  fi
  if [ -n "${HOME:-}" ] && LC_ALL=C grep -aF "$HOME" "$binary" >/dev/null 2>&1; then
    echo "Refusing to package $binary: build home is embedded" >&2
    exit 1
  fi
done

stage=$(mktemp -d "$root/dist-package.XXXXXX")
chmod 0755 "$stage"
# A fresh staging tree is used for each package build and retained for inspection.
make install "PROFILE=$profile" "DESTDIR=$stage"
mkdir -p "$stage/DEBIAN" "$stage/debian" "$root/dist"

cat > "$stage/debian/control" <<CONTROL
Source: cleanly
Section: utils
Priority: optional
Maintainer: $maintainer
Standards-Version: 4.7.0
Homepage: $homepage

Package: cleanly
Architecture: any
Description: Native application inspector and conservative uninstaller
CONTROL

# Let dpkg calculate the actual minimum versions from imported ELF symbols.
shlibs=$(cd "$stage" && dpkg-shlibdeps -O -S"$stage" \
  -e "$stage/usr/bin/cleanly" \
  -e "$stage/usr/libexec/cleanly-helper" \
  -e "$stage/usr/bin/cleanly-inspect")
case "$shlibs" in
  shlibs:Depends=*) ;;
  *) echo 'dpkg-shlibdeps did not return dependency metadata' >&2; exit 1 ;;
esac
dependencies=${shlibs#shlibs:Depends=}

# Build-only metadata does not belong in the installed package.
rm "$stage/debian/control"
rmdir "$stage/debian"

arch=$(dpkg --print-architecture)
cat > "$stage/DEBIAN/control" <<CONTROL
Package: cleanly
Version: $version
Section: utils
Priority: optional
Architecture: $arch
Maintainer: $maintainer
Homepage: $homepage
Depends: $dependencies, pkexec, dpkg
Description: Native application inspector and conservative uninstaller
 Inspect APT, Flatpak, Snap and AppImage ownership before removal.
 Preview release with guarded quarantine and restore.
CONTROL

# Keep dist deterministic: it contains only the package built by this invocation.
rm -f "$root"/dist/cleanly_*.deb "$root/dist/SHA256SUMS"
package="$root/dist/cleanly_${version}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$package"
(
  cd "$root/dist"
  sha256sum "$(basename "$package")" > SHA256SUMS
)

printf 'Package created at %s; staging tree preserved at %s\n' "$package" "$stage"
