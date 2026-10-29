"""Prepare private ACXi inputs and static translations from the user's IPA.

SPDX-License-Identifier: MPL-2.0
"""
import argparse
import hashlib
import importlib.metadata
import json
from pathlib import Path, PurePosixPath
import plistlib
import stat
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
GAME_SHA256 = "9ff36f1525925cd886490f6b0a1acaed4a510038e823e94b5f22d31d41f03873"
PREFIX = "Payload/ACE-Xi.app/"


def reject_link(path):
    for item in (path, *path.parents):
        if item.is_symlink() or (item.exists() and getattr(item.lstat(), "st_file_attributes", 0) & 0x400):
            raise ValueError("Refusing to write through a link or junction")


def extract(ipa, output):
    app = output / "game/ACE-Xi.app"
    with zipfile.ZipFile(ipa) as archive:
        executable = archive.read(PREFIX + "ACE-Xi")
        if hashlib.sha256(executable).hexdigest() != GAME_SHA256:
            raise ValueError("Expected the supported ACXi 1.1.3 executable; the supplied IPA does not match")
        info = plistlib.loads(archive.read(PREFIX + "Info.plist"))
        if info.get("CFBundleIdentifier") != "com.namconetworks.AceCombatXi":
            raise ValueError("Unexpected application identifier")
        entries = [entry for entry in archive.infolist() if entry.filename.startswith(PREFIX)]
        if sum(entry.file_size for entry in entries) > 1024 ** 3:
            raise ValueError("Unexpected app size")
        seen = set()
        for entry in entries:
            name = entry.filename[len(PREFIX):]
            relative = PurePosixPath(name)
            parts = relative.parts
            if not parts:
                continue
            if relative.is_absolute() or any(part in (".", "..") or ":" in part or "\\" in part for part in parts):
                raise ValueError("Unsafe IPA path")
            key = "/".join(parts).casefold()
            if key in seen:
                raise ValueError("Duplicate IPA path")
            seen.add(key)
            if stat.S_ISLNK(entry.external_attr >> 16):
                raise ValueError("IPA symbolic links are unsupported")
            target = app.joinpath(*parts)
            reject_link(target)
            if entry.is_dir():
                target.mkdir(parents=True, exist_ok=True)
                continue
            data = archive.read(entry)
            if target.exists():
                if not target.is_file() or target.read_bytes() != data:
                    raise ValueError(f"Existing private game file differs: {name}; use a fresh local directory")
                continue
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
    return app


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("ipa", type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "local")
    parser.add_argument("--reuse-generated", action="store_true")
    args = parser.parse_args()
    import capstone
    if importlib.metadata.version("capstone") != "5.0.9":
        raise RuntimeError("Install the pinned decoder using: python -m pip install -r requirements.txt")
    for name, expected in {
        'libgcc_s.1.dylib': 'd758fbf1ac952532955d62866bd02df55315c0599f64bdd978ff941fccbdcf83',
        'libstdc++.6.0.9.dylib': '22bb45b80a1e590ca6f2e0bd07581084ac56b347b0c6005b05ca5551ef27142a',
    }.items():
        if hashlib.sha256((ROOT / 'runtime/touchHLE_dylibs' / name).read_bytes()).hexdigest() != expected:
            raise ValueError('Support library identity mismatch: ' + name)
    reject_link(args.output)
    output = args.output.resolve()
    # Keep all proprietary inputs and results inside the ignored local tree.
    if output != (ROOT / "local").resolve():
        raise ValueError("Private generation output must be this project's local directory")
    output.mkdir(parents=True, exist_ok=True)
    app = extract(args.ipa, output)
    with args.ipa.open("rb") as stream:
        ipa_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    source_hash = hashlib.sha256()
    generator_files = [ROOT / 'tools/prepare.py', ROOT / 'tools/apply_port_hooks.py']
    generator_files += list((ROOT / 'tools/aot').glob('*.py')) + list((ROOT / 'tools/aot').glob('*.c'))
    generator_files += [ROOT / 'runtime/src/cpu/aot_wrapper/vfp' / name
                        for name in ('acxi_softfloat.c', 'acxi_softfloat.h', 'platform.h')]
    generator_files += [ROOT / 'runtime/vendor/softfloat/required-sources.txt']
    for path in sorted(generator_files):
        source_hash.update(path.relative_to(ROOT).as_posix().encode())
        source_hash.update(path.read_bytes())
    identity = {"ipa_sha256": ipa_hash, "generator_sha256": source_hash.hexdigest()}
    marker = output / "generation.json"
    generated = output / "generated"
    if args.reuse_generated and marker.is_file():
        recorded = json.loads(marker.read_text())
        if recorded.get("inputs") == identity:
            for name, digest in recorded["files"].items():
                path = generated / name
                if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
                    raise ValueError("Generated output changed; rerun without --reuse-generated")
            print("Verified existing private translations")
            return
    def run(script, *arguments):
        subprocess.run([sys.executable, str(ROOT / "tools" / script), *map(str, arguments)], check=True)
    run("aot/generate_aot.py", output / "generated-base", "--ipa", args.ipa.resolve(), "--runtime", ROOT / "runtime", "--full")
    run("aot/generate_blocks.py", output / "generated-base", generated)
    run("apply_port_hooks.py", generated)
    files = {path.name: hashlib.sha256(path.read_bytes()).hexdigest()
             for path in sorted(generated.iterdir()) if path.is_file()}
    marker.write_text(json.dumps({"inputs": identity, "files": files}, indent=2) + "\n")
    print("Private game and native translation prepared under local/")


if __name__ == "__main__":
    main()
