# Source export validation — 29 October 2026

The source project was prepared independently of the private playable install.
Its build scripts use explicit input/toolchain arguments and project-relative
output paths. No original game data or generated game instructions are part of
the source release.

## Completed checks

- Generated 319 native translation pages from the supplied matching IPA and
  applied the production port hooks. All 319 pages exactly match the previously
  tested translation; six support files match after ignoring line endings and
  final blank lines. Generation preserves 323,789 instruction labels.
- Built a release executable from the exported runtime, vendored dependencies
  and newly generated translation. The native build compiled 378 C sources from
  an empty native object cache. The checked tools were Rust 1.99.1 GNU host,
  LLVM-MinGW clang 23.1.4, CMake 4.4.4, Ninja 1.13.2 and Capstone distribution 5.0.9.
- **68 runtime tests passed**, including real SDL virtual-controller rumble
  callbacks and both successful and failed settings-file replacement paths.
  Temporary files stayed inside the private build directory.
- The private staged EXE completed `--help` and `--copyright` with exit code 0
  using only Windows system directories on PATH. Its hash was
  `961ACD6D391844190EB530C5BD6EDF3B517ECF66EB90E4D390EB67D6B3E89293`.
- Verified the pinned ANGLE archive and every selected library/license checksum
  using the cached archive. No new graphics dependency was substituted.
- Nine pipeline checks covered valid/repeated extraction, traversal, absolute
  paths, drive/backslash paths, archive symlinks, wrong game identity, unexpected
  compiled payloads and credential-pattern rejection.
- The source audit found no matching original IPA assets, personal paths,
  credential patterns or links in the selected source files. Dependency files
  match their recorded export hashes. The source ZIP passed CRC, exact member-list
  and SHA-256 checks for every archived source member.
- The normal installed EXE, settings and save were unchanged. Private generation,
  build, test and player outputs were moved out of the publication directory
  after validation; they are excluded from Git and the source ZIP.

The only runtime-file edit after the successful build removed an incorrect extra
license label from the SoftFloat-derived platform header, preserving its original
notice. That comment-only correction changes no executable code. The final
generation fingerprint includes the VFP wrapper files and required-source list;
generation reuse and support-library identity checks passed.

## Scope

This pass validates the source preparation, build and packaging workflow. It did
not repeat flight gameplay, physical-controller feel or audio listening tests.
The earlier bounded game checks and their separate executable hash remain in
[VERIFICATION.md](VERIFICATION.md). A full campaign and other hardware remain
outside the evidence.
