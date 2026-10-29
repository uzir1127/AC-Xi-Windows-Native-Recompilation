# Publishing the source project

These instructions are for maintaining the repository. To build and play the
game, follow [BUILDING.md](BUILDING.md) instead.

The public upload is this source tree or a source ZIP made by
`tools/audit_release.py`. Run from the root:

```powershell
python tools/audit_release.py --game-ipa 'D:\Games\ACXi\game.ipa' `
  --archive '..\acxi-native-source.zip'
git status --short
```

The optional IPA argument is read only and never added to the archive. The audit
checks paths, dependency hashes, links, credential/private-path patterns and
unexpected compiled/archive payloads. It compares candidate file contents against
the IPA, writes `SOURCE_MANIFEST.json`, verifies ZIP CRC and hashes every archived
member. The archive output must be outside the project and must not already exist.

`local/`, generated pages, caches, saves, settings, original game files and the
private enhanced assets are excluded from the source tree and ZIP. Git ignores
private outputs, but the archive is built from its own checked file list rather
than assuming .gitignore is sufficient.

Building and staging create `local/player` on the builder's own PC. This folder
contains game assets copied from their IPA and an EXE containing recompiled game
instructions. It is not included in this repository or the source ZIP. Keep it
local; each player builds their own copy from their own IPA.

The build, staging and auditing scripts do not upload anything to GitHub.
