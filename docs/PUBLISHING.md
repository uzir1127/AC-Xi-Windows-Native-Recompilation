# Publishing the source project

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

Do not upload the EXE or `local/player`: the EXE embeds recompiled original game
instructions and the player directory includes extracted game data. Users build
their own executable from their matching IPA with the included scripts.

The intended GitHub destination is `https://github.com/uzir1127/acxi-native`.
The separate private runtime checkout still has its upstream touchHLE remote;
this export must never be pushed to that upstream remote. No additional GitHub
upload is performed by any build, staging or auditing script.
