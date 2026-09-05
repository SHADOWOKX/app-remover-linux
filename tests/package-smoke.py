#!/usr/bin/env python3
"""Read-only package acceptance: metadata, checksum, permissions, policy and binaries."""

import hashlib
import io
import pathlib
import subprocess
import tarfile
import xml.etree.ElementTree as ET

root = pathlib.Path(__file__).resolve().parents[1]
dist = root / "dist"
packages = sorted(dist.glob("cleanly_*.deb"))
assert len(packages) == 1, f"expected exactly one .deb in dist, found {len(packages)}"
package = packages[0]


def control_field(name: str) -> str:
    return subprocess.run(
        ["dpkg-deb", "-f", str(package), name],
        check=True,
        stdout=subprocess.PIPE,
        text=True,
        timeout=10,
    ).stdout.strip()


assert control_field("Package") == "cleanly"
assert control_field("Version")
assert control_field("Architecture")
assert control_field("Homepage") == "https://github.com/SHADOWOKX/app-remover-linux"
assert control_field("Maintainer") == "SHADOWOKX"
assert control_field("Depends"), "package dependency metadata is empty"

checksum_file = dist / "SHA256SUMS"
checksum_parts = checksum_file.read_text(encoding="utf-8").strip().split()
assert len(checksum_parts) == 2, "invalid SHA256SUMS format"
expected_hash, expected_name = checksum_parts
assert expected_name.lstrip("*") == package.name, "checksum references the wrong package"
actual_hash = hashlib.sha256(package.read_bytes()).hexdigest()
assert actual_hash == expected_hash, "package checksum mismatch"

archive = subprocess.run(
    ["dpkg-deb", "--fsys-tarfile", str(package)],
    check=True,
    stdout=subprocess.PIPE,
    timeout=15,
).stdout

with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
    entries = {entry.name.removeprefix("./"): entry for entry in tar.getmembers()}

    for entry in entries.values():
        assert entry.uid == entry.gid == 0, entry.name
        assert not entry.mode & 0o022, entry.name
        assert not entry.mode & 0o6000, entry.name
        assert not entry.issym() and not entry.islnk(), entry.name
        if entry.isdir():
            assert entry.mode == 0o755, (entry.name, oct(entry.mode))

    for name, path in [
        ("cleanly", "usr/bin/cleanly"),
        ("cleanly-inspect", "usr/bin/cleanly-inspect"),
        ("cleanly-helper", "usr/libexec/cleanly-helper"),
    ]:
        entry = entries[path]
        assert entry.mode == 0o755
        assert tar.extractfile(entry).read() == (root / "target/release" / name).read_bytes()

    policy = ET.fromstring(
        tar.extractfile(entries["usr/share/polkit-1/actions/io.github.cleanly.Cleanly.policy"]).read()
    )
    assert policy.findtext("action/defaults/allow_active") == "auth_admin"
    assert policy.findtext("action/defaults/allow_any") == "no"
    assert policy.findtext("action/annotate") == "/usr/libexec/cleanly-helper"
    assert "usr/share/doc/cleanly/copyright" in entries

print(
    "Package verified: metadata, checksum, root ownership, safe modes, "
    "fixed polkit boundary and exact release binaries."
)
