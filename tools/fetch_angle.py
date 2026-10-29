"""Fetch the pinned graphics dependency into ignored local storage.

SPDX-License-Identifier: MPL-2.0
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, help='Use an already downloaded pinned archive')
    args = parser.parse_args()
    manifest = json.loads((ROOT / "dependencies/angle.json").read_text())
    if args.archive:
        data = args.archive.read_bytes()
    else:
        request = urllib.request.Request(manifest["archive_url"], headers={"User-Agent": "ACXi-source-setup"})
        with urllib.request.urlopen(request, timeout=120) as response:
            data = response.read(128 * 1024 * 1024 + 1)
    if hashlib.sha256(data).hexdigest() != manifest["archive_sha256"]:
        raise ValueError("ANGLE archive checksum mismatch")
    target = ROOT / "local/angle"
    for item in (target, *target.parents):
        if item.is_symlink() or (item.exists() and getattr(item.lstat(), "st_file_attributes", 0) & 0x400):
            raise ValueError("Refusing to write through a link or junction")
    target.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(io.BytesIO(data)) as archive:
        members = {name: 'angle/out/Release/' + name for name in ('libEGL.dll', 'libGLESv2.dll', 'vulkan-1.dll')}
        members.update({'LICENSE.ANGLE.txt': 'angle/LICENSE', 'LICENSE.Vulkan-Loader.txt': 'angle/VULKAN_LOADER_LICENSE.txt'})
        for name, member in members.items():
            payload = archive.read(member)
            if hashlib.sha256(payload).hexdigest() != manifest["files"][name]["sha256"]:
                raise ValueError("ANGLE file checksum mismatch: " + name)
            (target / name).write_bytes(payload)
    print("Pinned ANGLE files verified under local/angle")


if __name__ == "__main__":
    main()
