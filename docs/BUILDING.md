# Building on Windows x64

Builds use your own ACXi 1.1.3 IPA. Extraction, generated C, C objects, Rust outputs,
the playable installation and saves all stay inside the ignored `local/` tree.
The normal installed game/save profile is not used by these scripts.

## Prerequisites

- Windows x64 and a hardware Vulkan graphics driver for running the game.
- Python 3.11 or later and the distribution pinned in `requirements.txt`.
- GNU-host Rust with the `x86_64-pc-windows-gnullvm` target. The checked toolchain
  is Rust 1.99.1 with host `x86_64-pc-windows-gnu`. The documented setup avoids
  depending on a Visual Studio linker for Cargo's host build scripts.
- LLVM-MinGW UCRT x64, including clang, clang++, llvm-ar, windres, libc++ and
  libunwind. The development build uses the 20261020 distribution, clang 23.1.4.
- CMake and Ninja on PATH. The checked tools are CMake 4.4.4 and Ninja 1.13.2.
- An internet connection for the initial Python/Rust dependency setup and the
  pinned ANGLE download, unless those dependencies are already cached.

SDL, OpenAL Soft, stb_image, PVRTDecompress and the required SoftFloat sources
are vendored with their notices. Upstream ARM support libraries and fonts are
also included with touchHLE's source/provenance documentation. These are
third-party dependencies, not files taken from the ACXi IPA.

Select the checked GNU-host toolchain for this project, then install its target:

```powershell
rustup toolchain install 1.99.1-x86_64-pc-windows-gnu --profile minimal
rustup override set 1.99.1-x86_64-pc-windows-gnu
rustup target add x86_64-pc-windows-gnullvm
python -m pip install -r requirements.txt
```

If Windows' `python` command opens the Store, use the actual Python executable
path with `build.ps1 -Python`. No scripts install or alter a system toolchain.

## Compile from your own IPA

From the project root:

```powershell
.\build.ps1 -IpaPath 'D:\Games\ACXi\game.ipa' `
  -LlvmMingwRoot 'C:\Tools\llvm-mingw' -RunTests
```

`-Python` and `-Cargo` can select explicit executable paths. `-Jobs` controls
parallel compilation, default 6. Add `-Offline` only after the locked Cargo
dependencies have been fetched. The script respects your existing CARGO_HOME and
RUSTUP_HOME and does not contain a developer-specific compiler path.

The build first validates the executable identity and securely extracts the app
to `local/game/ACE-Xi.app`. It generates the full ARM instruction translation,
applies the page-control-flow transformation and the port's hooks, then compiles
the native Windows executable. Unknown code retains explicit traps; no ARM
interpreter/JIT fallback is built.

Generation can take some time, and the C compilation is substantial. For later
builds, `-ReuseGenerated` verifies the IPA, generator identity and generated-file
hashes before reusing translations. Omit that switch after changing the
translator or hooks. A source change never requires altering the original IPA.

The EXE is `local/target/x86_64-pc-windows-gnullvm/release/touchHLE.exe`.
Its filename retains the runtime crate's upstream name; the playable staging step
copies it as `ACXiNative.exe`.

## Graphics dependency and private playable install

```powershell
python tools/fetch_angle.py
python tools/stage_local.py --compiler 'C:\Tools\llvm-mingw' --angle local/angle
.\local\player\ACXiNative.exe
```

The download is pinned by archive and file SHA-256 values in
`dependencies/angle.json`. It selects only the required ANGLE/Vulkan libraries
and license notices. If you already have those exact verified files, pass their
directory to `stage_local.py --angle` instead of downloading again.
An already downloaded archive can also be verified and unpacked with
`python tools/fetch_angle.py --archive 'D:\Downloads\angle.zip'`.

The staging script uses original assets, mobile buttons Off and manual settings
save for a new local profile. Re-running it preserves that profile's existing
settings and save directory. It does not copy the private enhanced asset pack.

The source project does not promise compatibility with arbitrary ACXi versions,
encrypted inputs, other games, other operating systems or other graphics APIs.
