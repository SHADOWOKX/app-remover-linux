#!/bin/sh
set -eu
profile=${1:-release}
case "$profile" in release|debug) ;; *) echo 'Expected release or debug' >&2; exit 1;; esac
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
stage=$(mktemp -d "$root/dist-package.XXXXXX")
chmod 0755 "$stage"
# Only a new, private staging tree is used; retained for inspection after packaging.
make install "PROFILE=$profile" "DESTDIR=$stage"
mkdir -p "$stage/DEBIAN" "$stage/debian" "$root/dist"
cat > "$stage/debian/control" <<'SOURCE'
Source: cleanly
Section: utils
Priority: optional
Maintainer: Cleanly contributors <cleanly@example.invalid>
Standards-Version: 4.7.0

Package: cleanly
Architecture: any
Description: Native application inspector and conservative uninstaller
SOURCE
# Let dpkg calculate the actual minimum versions from imported ELF symbols.
shlibs=$(cd "$stage" && dpkg-shlibdeps --package=cleanly -O -S"$stage" -e "$stage/usr/bin/cleanly" -e "$stage/usr/libexec/cleanly-helper" -e "$stage/usr/bin/cleanly-inspect")
dependencies=${shlibs#shlibs:Depends=}
# Build-only metadata does not belong in the installed package.
rm "$stage/debian/control"
rmdir "$stage/debian"
arch=$(dpkg --print-architecture)
cat > "$stage/DEBIAN/control" <<CONTROL
Package: cleanly
Version: 0.1.0
Section: utils
Priority: optional
Architecture: $arch
Maintainer: Cleanly contributors <cleanly@example.invalid>
Depends: $dependencies, pkexec, dpkg
Description: Native application inspector and conservative uninstaller
 Inspect APT, Flatpak, Snap and AppImage ownership before removal.
 Preview release with guarded quarantine and restore.
CONTROL
dpkg-deb --root-owner-group --build "$stage" "$root/dist/cleanly_0.1.0_${arch}.deb"
printf 'Package created; staging tree preserved at %s\n' "$stage"
