#!/usr/bin/env python3
"""Read-only package acceptance: metadata, permissions, policy and exact built binaries."""
import io
import pathlib
import subprocess
import tarfile
import xml.etree.ElementTree as ET

root = pathlib.Path(__file__).resolve().parents[1]
package = root / "dist/cleanly_0.1.0_amd64.deb"
archive = subprocess.run(["dpkg-deb", "--fsys-tarfile", str(package)],
                         check=True, stdout=subprocess.PIPE, timeout=15).stdout
with tarfile.open(fileobj=io.BytesIO(archive), mode="r:") as tar:
    entries = {entry.name.removeprefix("./"): entry for entry in tar.getmembers()}
    for entry in entries.values():
        assert entry.uid == entry.gid == 0, entry.name
        assert not entry.mode & 0o022, entry.name
        assert not entry.mode & 0o6000, entry.name
        assert not entry.issym() and not entry.islnk(), entry.name
        if entry.isdir():
            assert entry.mode == 0o755, (entry.name, oct(entry.mode))
    for name, path in [("cleanly", "usr/bin/cleanly"),
                       ("cleanly-inspect", "usr/bin/cleanly-inspect"),
                       ("cleanly-helper", "usr/libexec/cleanly-helper")]:
        entry = entries[path]
        assert entry.mode == 0o755
        assert tar.extractfile(entry).read() == (root / "target/release" / name).read_bytes()
    policy = ET.fromstring(tar.extractfile(entries[
        "usr/share/polkit-1/actions/io.github.cleanly.Cleanly.policy"]).read())
    assert policy.findtext("action/defaults/allow_active") == "auth_admin"
    assert policy.findtext("action/defaults/allow_any") == "no"
    assert policy.findtext("action/annotate") == "/usr/libexec/cleanly-helper"
    assert "usr/share/doc/cleanly/copyright" in entries
print("Package verified: root ownership, safe modes, fixed polkit boundary, exact release binaries.")
