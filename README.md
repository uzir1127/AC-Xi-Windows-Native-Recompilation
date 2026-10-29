# Ace Combat Xi — Windows native recompilation

A Windows x64 development port of **Ace Combat Xi: Skies of Incursion 1.1.3**. The goal is to preserve the original game behavior and timing while making the game playable with modern PC controls.

The original ARM game code is statically recompiled into native x64 code through generated C. The runtime adapts touchHLE components for the iOS framework services used by the game; it does not interpret or JIT-translate ARM instructions at runtime. Rendering uses ANGLE's GLES frontend with its hardware Vulkan backend.

> **Development build:** This project has passed bounded live checks, but it is not a finished release. No runnable public build is attached to this repository. See [verification](VERIFICATION.md) for what was checked and what remains open.

## Current features

- Original game simulation timing retained: sampled gameplay ran at about 30 game FPS, presented at about 60 display FPS on the test PC. The simulation has not been uncapped.
- Keyboard, mouse and SDL controller support, including right-stick free look and the original in-game pitch-inversion option.
- Event-driven controller vibration for gunfire, missile launches and damage. The project owner reports that vibration works on their controller; device coverage is not established.
- F4 display and graphics settings, mobile-button visibility, keyboard/mouse options, manual settings save and optional auto-save.
- Mobile flight buttons default Off while the original HUD, menus and mobile layout remain available.
- F5 overlay for graphics API, hardware names, game/display frame rates, frame times and available system counters.
- Optional enhanced local texture, text and intro assets, with an Original mode. Game-derived assets are not included in a public package.

## Game data and launch

You must provide your own **Ace Combat Xi 1.1.3** game data. Do not redistribute the IPA, extracted game files, saves, or the private enhanced asset pack. This draft does not include a ready-to-run public download or a clean-clone build walkthrough.

The local development package accepts the matching IPA or an extracted `ACE-Xi.app`. Keep the runtime libraries and resource folders beside the executable. The exact public installation steps will be documented after a clean, redistributable package is prepared.

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

Menus use arrows/WASD, Enter to confirm and Escape to go back. The original in-game Controller settings remain available. Mouse flight captures the pointer from the flight view; sustained mouse-steering feel still needs hands-on validation.

## Settings

Use **F4 → Save settings**, or turn on **Auto-save settings**. Auto-save is Off by default. Mobile flight buttons are Off by default. Window size, fullscreen, internal resolution, filtering and asset mode can be saved and restored; consult the verification notes for the exact live checks completed.

## Status and limitations

The latest packaged candidate completed bounded checks in Mission 1 and Extra Mission, settings and resolution changes, pause/resume, weapons, menus, hidden and visible mobile controls, and normal shutdown. No crash or Vulkan context-loss error appeared in those test logs. That is evidence for the tested sessions, not a guarantee of stability in every mission, driver or PC.

A full campaign clear, all missions and aircraft, sustained mouse steering, audio listening/synchronization, controller-device variations and performance on low-end Intel laptops have not been certified. See [VERIFICATION.md](VERIFICATION.md).

About 30 game FPS and 60 display FPS means the host presents some game frames more than once; it does not mean the simulation produces 60 distinct frames per second.

## Credits and licensing

Original game: Project Aces / Namco Bandai. Runtime foundation: touchHLE and its contributors. Graphics: ANGLE with the Vulkan backend. Other components and their licenses are listed in the local development package. A public source or binary distribution still needs its corresponding source/license and packaging review. This is an unofficial fan project.

