# Branch log: feat/gui

## Merged 2026-10-12

Merged into `main` as a fast-forward. BoostDeck is now usable from its
window: boost on and off, a dB slider, live meters, an output picker, the
engine's questions as yes/no prompts, and a setup screen when VB-Cable is
missing, all driving the control thread from step 7 (coding-order step 8).

### Commits

| Commit | Subject |
| --- | --- |
| `880cec9` | Add the GUI: status and boost toggle, slider, meters, output picker, setup screen |
| `212029f` | Name the lost output in the fallback log line |
| `ccc457f` | Tick step 4: the boost is audible, confirmed by ear from the slider |
| (this commit) | Document feat/gui for the main merge |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +2 -2 |
| `crates/app/src/main.rs` | modified, +8 -3 |
| `crates/app/src/settings.rs` | modified, +95 -0 |
| `crates/app/src/state.rs` | modified, +309 -89 |
| `crates/app/src/ui.rs` | modified, +310 -24 |
| `crates/engine/examples/common/mod.rs` | modified, +3 -24 |
| `crates/engine/examples/control.rs` | modified, +5 -0 |
| `crates/engine/src/console.rs` | added, +32 -0 |
| `crates/engine/src/control.rs` | modified, +36 -1 |
| `crates/engine/src/lib.rs` | modified, +1 -0 |
| `docs/feat-gui-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass (a first run failed with "Access is denied" because the user's BoostDeck window was still open and locked `boostdeck.exe`; passed once it was closed) |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 81 tests (app 6, dsp 25 + 2 no-alloc, engine 48) |
| Window driven through UI Automation, checked by screenshot | Boost on: default to CABLE Input, Running, meters moving with `Ring01.wav` looping; slider to +6 dB: label 200%, Out about 6-7 dB over In; Boost off and closing the window while on: Realtek restored, record cleared |
| Kill while boosting (`taskkill /F`), relaunch | default left on CABLE Input with the record set; relaunch restored the Realtek and showed the "did not shut down cleanly" notice |
| Default moved to ARZOPA while boosting | prompt "Boost it instead?" shown; Boost it played to ARZOPA and the picker kept the choice |
| User, from the window | boost on/off three times; Bluetooth headphones picked, then switched off: fell back to the Realtek in the same second; default changed by hand: paused, resumed on Boost on; sample rate 44100, 48000, 96000, 192000 and back to 48000 Hz: rebuilt each time at the new rate |
| User, by ear | moving the slider made the speakers louder and quieter: step 4's "the boost is audible" |

### Notes

- Steps 4 and 8 are ticked in the README. Step 4 was waiting only on a
  listening check, which the user did from this window's slider.
- The setup screen was not seen live: it needs VB-Cable missing or a cable
  endpoint disabled. It shows whenever the engine's `DeviceSnapshot` reports
  a `cable_problem`.
- Settings are one `SharedSettings` handle - an `Arc<Mutex<Settings>>` that
  saves on every change - and it is also the engine's `RoutingJournal`, so
  the UI and the control thread never save over each other.
- The control thread now sends a `DeviceSnapshot` at start, after every
  device notification and on `RefreshDevices`, so the UI never touches COM.
- The Ctrl+C handler moved from the examples to `engine::console`; the app
  installs it and turns Ctrl+C into a normal window close, so `on_exit`
  restores the default (the follow-up from feat/routing).
- Gain starts at 0 dB on every launch and is not saved: saving it belongs
  with step 10's safe start, which caps a saved gain at +6 dB.
- Not in this step: the presets panel (step 9) and the max-boost unlock
  dialog and panic button (step 10).
- The guide's on_exit sketch sends Shutdown and waits for an event; here
  dropping `Engine` sends Shutdown and joins the control thread, which
  restores before returning.
