# Port changes and source export

The runtime starts from touchHLE commit
`ca1de89c578e79746f33c1332e28efd106c334ba`. Upstream notices and contributor
credits remain in the files. ACXi port development and this export were prepared
27–29 October 2026. `dependencies/provenance.json` records the affected upstream
paths and dependency revisions.

The port replaces the CPU backend with an ahead-of-time native bridge and a
build-time ARM-to-C translator. It retains the iOS framework/ABI and memory
services used by the game. Unknown code stops explicitly; general ARM execution
fallback is absent. The unused app picker, Dynarmic bridge and desktop GL2
backend were removed from this game-specific fork.

The Windows host forces ANGLE hardware Vulkan, provides software movie decoding,
adds F4/F5 settings and telemetry, and snapshots game frames on the GPU where
supported. The packaged swapchain workaround disables ANGLE's deferred memory
allocation option before graphics initialization and retains the EGL surface
during native window changes.

Port hooks integrate native flight buttons, camera/free-look input, mobile UI
visibility, menu navigation, rumble events and the requested original bonus
content ownership flags. The original simulation timer and frame-rate limit
remain unchanged.

The publication export adds path-independent preparation/build/staging scripts,
validates supplied game identity, keeps generated game instructions private,
retains dependency provenance/notices, and creates source archives from an
audited file list. This export does not include the game's translated C,
compiled EXE or enhancement assets.
