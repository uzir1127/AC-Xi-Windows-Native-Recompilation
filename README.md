# Ace Combat Xi — Windows native recompilation

Source project for the Windows x64 port of **Ace Combat Xi: Skies of Incursion 1.1.3**.
The original ARM instructions are translated to C during the build and compiled
to native x64 code. Adapted touchHLE components provide the iOS services the game
uses. Graphics pass through ANGLE's GLES frontend to hardware Vulkan.

**You need your own Ace Combat Xi 1.1.3 IPA to build and play.** This repository
contains the port's source code, build tools and dependencies. It does not include
the original game, its assets or a compiled game executable.

## Build and play

Follow [BUILDING.md](docs/BUILDING.md) to install the required Windows build tools.
Then run these commands, replacing the example paths with your own. The build
checks that your IPA is the supported version.

```powershell
python -m pip install -r requirements.txt
.\build.ps1 -IpaPath 'D:\Games\ACXi\game.ipa' -LlvmMingwRoot 'C:\Tools\llvm-mingw' -RunTests
python tools/fetch_angle.py
python tools/stage_local.py --compiler 'C:\Tools\llvm-mingw' --angle local/angle
.\local\player\ACXiNative.exe
```

These commands create the playable game in `local/player` on your own PC, using
your IPA. That folder is created during the build; it is not part of this repository.

## Features

- Original game timing retained, with about 30 game FPS and 60 host presentations
  per second observed on the tested PC. There is no frame-rate unlock.
- Keyboard, mouse and SDL controller input, in-game pitch inversion and
  right-stick free look.
- Controller vibration for gunfire, missile launches, damage, destruction,
  afterburner and stall feedback. Hardware vibration was reported working by the
  project owner; device coverage remains limited.
- F4 settings for output size/fullscreen, internal resolution, filtering,
  mobile-button visibility and keyboard/mouse options. Manual Save and optional
  Auto-save are available. Internal resolution/filtering/asset mode apply after
  restart; output-size/fullscreen changes are live.
- Mobile flight buttons default Off, with the original HUD and menus retained.
- F5 hardware/API, game/display FPS and frame-time overlay.
- Optional enhanced-asset loading code. The private enhanced texture, text and
  movie pack is excluded; the staging script selects original assets by default.

## Controls

| Action | Keyboard | Controller |
| --- | --- | --- |
| Pitch / roll | WASD or arrows | Left stick |
| Yaw | Q / E | LB / RB |
| Throttle / brake | Space or Shift / Ctrl | RT / LT |
| Gun / missile | J / L or Enter | A / B |
| Target / radar | T / M or Tab | Y / View |
| Camera cycle | V | R3 |
| Free look | Numpad 4/6/8/2 | Right stick |
| Level flight | G | L3 |
| Pause | Escape / F1 | Start |
| Settings | F4 | View + Start |

Menus use arrows/WASD and Enter/Escape. Pitch inversion for controllers remains
in the original in-game Controller menu. F4 inversion applies to keyboard/mouse.
Pause the original game before spending time in F4: the settings overlay itself
does not pause the simulation. Settings save through F4; Auto-save defaults Off.

## Status

This is a development project. Bounded live checks reached Mission 1 and Extra
Mission and covered combat, pause/resume, menus, saves, F4/F5 and window changes.
The tested candidate showed no recurrence of the prior Vulkan crash in those
sessions. [Verification notes](docs/VERIFICATION.md) state the exact coverage.

Full campaign completion, all aircraft/missions, sustained mouse steering feel,
audio listening/sync, controller variations and low-end Intel laptop performance
remain unverified. The tested PC has an i7-9750H and GTX 1650. The clean source
build validation is recorded separately in [SOURCE_VALIDATION.md](docs/SOURCE_VALIDATION.md).

## Credits

Original game: Project Aces / Namco Bandai. Runtime foundation: touchHLE and its
contributors. Graphics: ANGLE with Vulkan. See [THIRD_PARTY.md](THIRD_PARTY.md)
and the retained notices for dependency origins and licenses. Original game
content is not covered by the source project's license. This is an unofficial
fan project.
