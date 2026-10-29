"""Audit and optionally archive only the publishable source tree.

SPDX-License-Identifier: MPL-2.0
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import zipfile

ROOT = Path(__file__).resolve().parents[1]
ROOT_FILES = {"README.md", "LICENSE", "THIRD_PARTY.md", "build.ps1", "requirements.txt",
              ".gitignore", ".gitattributes", "SOURCE_MANIFEST.json"}
RUNTIME_FILES = {"Cargo.toml", "Cargo.lock", "build.rs", "LICENSE", "OPTIONS_HELP.txt",
                 "touchHLE_default_options.txt", "gamecontrollerdb.txt", "LICENSE.GameControllerDB.txt"}
SOURCE_SUFFIXES = {".rs", ".c", ".cpp", ".h", ".toml"}
PRIVATE_DIRS = {"local", ".git", "runtime/target", "runtime/hd-assets", "runtime/touchHLE_sandbox", "runtime/touchHLE_apps"}
PRIVATE_FILES = {"runtime/touchHLE_log.txt", "runtime/acxi-settings.ini", "runtime/touchHLE_options.txt", "runtime/DUMP.txt"}
SECRET = re.compile(rb"(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,}|-----BEGIN (?:RSA |OPENSSH |EC )?PRIVATE KEY-----)")
PRIVATE_PATH = re.compile(rb"(?i)(?:C:[/\\]+Users[/\\]+Uzir|uzir1127@gmail\.com)")


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_files():
    files = []
    for directory, folders, names in os.walk(ROOT, followlinks=False):
        relative = Path(directory).relative_to(ROOT)
        folders[:] = [name for name in folders if name != "__pycache__" and
                      (relative / name).as_posix() not in PRIVATE_DIRS]
        for name in folders + names:
            path = Path(directory) / name
            if path.is_symlink() or getattr(path.lstat(), "st_file_attributes", 0) & 0x400:
                raise ValueError("Link in source tree: " + path.relative_to(ROOT).as_posix())
        for name in names:
            relative_name = (relative / name).as_posix()
            if (name.endswith(".pyc") or relative_name in PRIVATE_FILES or
                (relative.as_posix() == 'runtime' and name.startswith('touchHLE_wallpaper.'))):
                continue
            files.append(Path(directory) / name)
    return sorted(files)


def audit():
    vendor = json.loads((ROOT / "dependencies/vendor-files.json").read_text())
    files = source_files()
    manifest = {}
    for path in files:
        name = path.relative_to(ROOT).as_posix()
        parts = Path(name).parts
        if any(part in (".git", "AGENTS.md", "CLAUDE.md", "hd-assets", "touchHLE_sandbox", "touchHLE_apps") for part in parts):
            raise ValueError("Private or unrelated path: " + name)
        if name in vendor:
            if digest(path) != vendor[name]:
                raise ValueError("Vendored dependency differs from exported upstream bytes: " + name)
        else:
            allowed = (name in ROOT_FILES or
                       (parts[0] in ("docs", "licenses", "dependencies") and path.suffix in (".md", ".txt", ".json")) or
                       (parts[0] == "tools" and path.suffix in (".py", ".c")) or
                       (name.startswith("runtime/src/") and path.suffix in SOURCE_SUFFIXES) or
                       (name.startswith("runtime/.cargo/") and path.suffix == ".toml") or
                       (len(parts) == 2 and parts[0] == "runtime" and parts[1] in RUNTIME_FILES))
            if not allowed:
                raise ValueError("File outside source allowlist: " + name)
            if re.fullmatch(r"page_[0-9a-f]+\.c", path.name) or path.name in ("acxi_aot.h", "sources.txt", "Save.dat"):
                raise ValueError("Generated game code or save in source tree: " + name)
            data = path.read_bytes()
            if data.startswith((b"MZ", b"PK\x03\x04", b"\x7fELF")):
                raise ValueError("Unexpected compiled/archive payload: " + name)
            if SECRET.search(data) or PRIVATE_PATH.search(data):
                raise ValueError("Credential or personal path pattern: " + name)
        if name != "SOURCE_MANIFEST.json":
            manifest[name] = {"bytes": path.stat().st_size, "sha256": digest(path)}
    missing = set(vendor) - set(manifest)
    if missing:
        raise ValueError("Missing vendored dependency files: " + repr(sorted(missing)[:5]))
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path)
    parser.add_argument("--game-ipa", type=Path, help="Optional private comparison; this input is never archived")
    args = parser.parse_args()
    manifest = audit()
    matches = []
    if args.game_ipa:
        hashes = {value["sha256"]: name for name, value in manifest.items() if value["bytes"] > 128}
        with zipfile.ZipFile(args.game_ipa) as archive:
            for entry in archive.infolist():
                if entry.filename.startswith("Payload/") and not entry.is_dir() and entry.file_size > 128:
                    value = hashlib.sha256(archive.read(entry)).hexdigest()
                    if value in hashes:
                        matches.append(hashes[value])
        if matches:
            raise ValueError("Original game data found: " + repr(matches))
    (ROOT / "SOURCE_MANIFEST.json").write_bytes((json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode())
    result = {"files": len(manifest), "source_bytes": sum(v["bytes"] for v in manifest.values()),
              "original_game_file_matches": len(matches) if args.game_ipa else None,
              "private_outputs_excluded": True, "reparse_links": 0}
    if args.archive:
        target = args.archive.resolve()
        if target.is_relative_to(ROOT):
            raise ValueError("Write the source archive outside the project tree")
        if target.exists():
            raise ValueError("Archive already exists; use a new output name")
        with zipfile.ZipFile(target, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as archive:
            for path in source_files():
                archive.write(path, "acxi-native/" + path.relative_to(ROOT).as_posix())
        with zipfile.ZipFile(target) as archive:
            if archive.testzip() is not None:
                raise ValueError("Archive CRC failed")
            expected = set(manifest) | {"SOURCE_MANIFEST.json"}
            names = {name.removeprefix("acxi-native/") for name in archive.namelist()}
            if names != expected:
                raise ValueError("Archive file list mismatch")
            for name, value in manifest.items():
                if hashlib.sha256(archive.read("acxi-native/" + name)).hexdigest() != value["sha256"]:
                    raise ValueError("Archive content mismatch: " + name)
        result["archive_sha256"] = digest(target)
        result["archive_bytes"] = target.stat().st_size
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
