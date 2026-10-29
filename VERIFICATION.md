# Verification — 29 October 2026

**Candidate status: bounded functional checks completed; development build remains under review.** This is not full-campaign or broad hardware certification.

Candidate executable SHA-256:

`E67B3B6B70F5438B79A42DB9EF3154F16CA978613DA083B43C9BC9A5939DC4CA`

## Current packaged candidate

The hash above matches the local `ACXiNative.exe` and `build-manifest.json`. No runtime source or executable was changed during the final functional check.

### Recent automated and diagnostic results

- **68 release runtime tests** passed for the unchanged build, covering settings persistence/error handling, input, graphics state, audio/time helpers and rumble lifecycle. They were not rerun during the final live check.
- SDL virtual-controller callbacks checked rumble start, stop and recovery after a simulated transient failure. The project owner separately reports successful vibration on their physical controller.
- **140** compiled rumble-hook comparisons passed; **234** native input scenarios and **22** view-action checks passed.
- **108** camera-state comparisons, **6** analog-axis cases and **10** camera-geometry cases passed.
- **67** content/save initialization cases passed.
- **76** isolated ANGLE/Vulkan graphics and filtering checks passed.
- The final asset audit matched all **1,942** original app files to the provided IPA and checked **376** indexed enhanced PNGs plus the enhanced intro checksum. The enhanced assets remain local and are not part of this draft.

## Final live functional check

The same packaged EXE completed four launches totaling about **46.8 minutes**, each closed normally with exit code 0. Much of the time involved attract playback, menus and paused inspection; it was not 46.8 minutes of continuous flight. A separate crash regression used three launches totaling about **25.2 minutes** and observed ten fullscreen changes, also with normal exits. These bounded runs found no crash, fatal graphics error, Vulkan device loss or EGL context loss in their logs. They do not establish that future sessions cannot fail.

Live routes reached the Campaign Continue path, Mission 1 briefing/tutorial/hangar/flight, and the Extra Mission briefing/hangar/flight. Checks included aircraft selection, moving terrain and enemies, HUD/radar/AWACS, gun and missile input, pause/resume, the original in-game pitch setting, visible and hidden mobile controls, F4 settings, F5 overlay, window-size/fullscreen changes and normal shutdown. The Extra Mission results/save screen returned to the Free Mission list, but no successful mission clear was achieved in this pass.

Sampled steady gameplay was about **29.9–30.1 game FPS / 59.9–60 display FPS** on an Intel Core i7-9750H and NVIDIA GeForce GTX 1650. The original game timing was unchanged. This was not a comparative performance benchmark or a low-end hardware test.

The game audio callback queued non-silent 48 kHz stereo PCM; this check did not include a listening or synchronization comparison. Automated mouse input did not establish sustained steering feel. The physical controller rumble result is user-reported; this audit did not measure motor feel or test multiple devices.

## Remaining coverage

- Full campaign completion, successful mission rewards/debriefing and every mission/aircraft/effect.
- Sustained hands-on mouse steering and right-stick feel across devices.
- Audio listening quality and sync against original iOS playback.
- Controller reconnect, device-specific USB/Bluetooth support and motor-strength feel.
- Performance and driver compatibility on Intel HD 520 / low-end ThinkPad hardware.
- Clean-clone build and public source/dependency/license audit.
- A redistributable runtime archive with verified contents.

## Distribution boundary

This is a local documentation preview only. No public repository, GitHub post, release or upload has been created. The private playable installation contains original game data and enhanced game-derived assets and must not be uploaded wholesale. The review folder itself contains only documentation and this HTML preview.

