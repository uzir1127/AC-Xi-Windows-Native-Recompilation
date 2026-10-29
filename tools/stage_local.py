"""Create a private playable install. Never use its output as a public ZIP.

SPDX-License-Identifier: MPL-2.0
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]


def reject_links(path):
    for item in (path, *path.parents):
        if item.is_symlink() or (item.exists() and getattr(item.lstat(), "st_file_attributes", 0) & 0x400):
            raise ValueError("Link or junction in staging path")
    for directory, folders, files in os.walk(path, followlinks=False):
        for item in [Path(directory), *[Path(directory) / name for name in folders + files]]:
            if item.is_symlink() or getattr(item.lstat(), "st_file_attributes", 0) & 0x400:
                raise ValueError("Link or junction in staging directory")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=Path, required=True)
    parser.add_argument("--angle", type=Path, required=True)
    args = parser.parse_args()
    target = ROOT / "local/player"
    reject_links(target)
    executable = ROOT / "local/target/x86_64-pc-windows-gnullvm/release/touchHLE.exe"
    game = ROOT / "local/game/ACE-Xi.app"
    if not executable.is_file() or not game.is_dir():
        raise RuntimeError("Run build.ps1 with your IPA first")
    angle = json.loads((ROOT / "dependencies/angle.json").read_text())
    for name in ("libEGL.dll", "libGLESv2.dll", "vulkan-1.dll"):
        if hashlib.sha256((args.angle / name).read_bytes()).hexdigest() != angle["files"][name]["sha256"]:
            raise ValueError("Unexpected ANGLE file: " + name)
    target.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(executable, target / "ACXiNative.exe")
    for name in ("libEGL.dll", "libGLESv2.dll", "vulkan-1.dll"):
        shutil.copyfile(args.angle / name, target / name)
    for name in ("libc++.dll", "libunwind.dll"):
        shutil.copyfile(args.compiler / "bin" / name, target / name)
    for name in ("touchHLE_fonts", "touchHLE_dylibs"):
        shutil.copytree(ROOT / "runtime" / name, target / name, dirs_exist_ok=True)
    for name in ("touchHLE_default_options.txt", "gamecontrollerdb.txt", "LICENSE", "LICENSE.GameControllerDB.txt"):
        shutil.copyfile(ROOT / "runtime" / name, target / name)
    for source in (ROOT / "licenses").iterdir():
        if source.is_file():
            shutil.copyfile(source, target / source.name)
    shutil.copyfile(ROOT / "runtime/vendor/softfloat/COPYING.txt", target / "LICENSE.SoftFloat.txt")
    shutil.copyfile(args.compiler / "LICENSE.TXT", target / "LICENSE.LLVM-Mingw.txt")
    shutil.copytree(game, target / "game/ACE-Xi.app", dirs_exist_ok=True)
    # Original assets are the available mode in the public source release.
    settings = target / "acxi-settings.ini"
    if not settings.exists():
        settings.write_text("hd_assets=false\ntouch_buttons=false\nauto_save=false\n")
    print("Private playable installation staged under local/player. Do not redistribute it.")


if __name__ == "__main__":
    main()
