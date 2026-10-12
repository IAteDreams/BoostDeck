# Branch log: feat/safety

## Merged 2026-10-13

Merged into `main` as a fast-forward. Every rule in the README's Safety
table now holds in the app: the +24 dB unlock sits behind a warning, a
launch never starts louder than +6 dB, a red 0 dB button is always in
reach, a new output starts at 0 dB, and a first-run notice explains the
risks (coding-order step 10).

### Commits

| Commit | Subject |
| --- | --- |
| `f91ce2d` | Add the safety rules: unlock dialog, safe start, panic button, first-run notice |
| (this commit) | Document feat/safety for the main merge |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +1 -1 |
| `crates/app/src/settings.rs` | modified, +9 -0 |
| `crates/app/src/state.rs` | modified, +186 -19 |
| `crates/app/src/ui.rs` | modified, +132 -2 |
| `docs/feat-safety-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 97 tests (app 8, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| First-run notice, in the window | shown as a modal over the window; I understand saved `safety_notice_seen: true` |
| Limiter always on | no switch exists in the window; the step 4 ceiling tests pass |
| Boost cap +12 dB | slider set to 30 stopped at +12 dB |
| Unlock to +24 dB | checkbox opened the warning; Cancel left the cap at +12 dB; Allow +24 dB let the slider reach +24 dB; unticking clamped 20 dB down to +12 dB |
| No sudden jumps | every gain change goes through `Shared::set_target_gain_db` and the 30 ms smoother (step 4 tests) |
| Safe start | with +12 dB saved, relaunch started at +6 dB with "Restore +12.0 dB / Keep this"; Restore set +12 dB |
| Panic button | red 0 dB button took 9 dB to 0 dB and saved 0 |
| Start at 0 dB on new output | boosting at +12 dB on the Realtek, switching the output to ARZOPA reset the gain to 0 dB with a notice |
| Never leave silence | the restore paths from steps 6 and 7, tested live then |

All window checks were driven through Windows UI Automation and read back
from the window's controls, `settings.json` and screenshots.

### Notes

- Step 10 is ticked in the README.
- The saved gain is `settings.json`'s `last_gain_db`, written when a change
  settles (end of a slider drag, a click or typed value, a preset load, the
  panic button, exit), not on every frame. Safe start reads it, replacing
  feat/presets' cap on the active preset's gain.
- An unanswered restore offer is not overwritten on exit, so the louder gain
  is offered again next launch.
- Start at 0 dB on a new output applies to a switch within the session (the
  picker, a fallback, a prompt). A switch that a preset load asked for keeps
  the preset's gain, since the user chose both together.
- The panic button goes through the 30 ms smoother like every other gain
  change; at 30 ms it is "at once" in practice and keeps "No sudden jumps".
- Not exercised live: giving up after five failed rebuilds and restoring;
  only single faked failures have been run.
- After testing, the user's settings were put back, with the safety notice
  reset to unseen so the user sees it on their next launch.
