# Branch log: feat/drift-correction

## Merged 2026-10-12

Merged into `main` as a fast-forward. The render thread now tracks how much
audio is waiting in the ring and, if the cable's clock and the output's clock
pull apart, drops or repeats one frame in a thousand to bring it back to the
20 ms target, so long sessions neither drift into crackles nor build up delay
(coding-order step 5).

### Commits

| Commit | Subject |
| --- | --- |
| `d96ac1e` | Add drift correction: drop or repeat a frame to hold the ring at target |
| (this commit) | Document feat/drift-correction for the main merge |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +5 -3 |
| `crates/dsp/src/drift.rs` | modified, +249 -5 |
| `crates/engine/examples/passthrough.rs` | modified, +50 -7 |
| `crates/engine/src/render.rs` | modified, +165 -17 |
| `crates/engine/src/rt.rs` | modified, +94 -2 |
| `docs/feat-drift-correction-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 63 tests (dsp 25 + 2 no-alloc, engine 36) |
| Simulated drift | +/-100 ppm for 2 h and +/-500 ppm for 10 min hold the average within +/-10 ms; +/-700-frame offsets are brought back; matched clocks get no corrections |
| Mutation check | triggering at the full 10 ms tolerance instead of 5 ms fails the simulations (one 10 ms packet step never crosses the trigger) |
| 2-hour soak at +6 dB from 75 s idle, looping `Ring01.wav`, Realtek at 48 kHz, `d96ac1e` | fill 960..960 frames in all 1,440 five-second readings (band 480..1440): held; 0 underruns, 0 overruns, 0 discontinuities; 0 frames dropped, 0 repeated; -15.2 dBFS in, -9.2 out in 1,436 readings |

### Notes

- Step 5 is ticked in the README.
- Correction triggers at half the tolerance (5 ms), not at the tolerance:
  drift arrives as whole 10 ms packet steps, so a full-tolerance trigger let
  one step sit just under it and did nothing until a second step reached
  20 ms. README updated.
- On this PC the cable and the Realtek never drifted: not one packet in two
  hours (under ~0.7 ppm), so the soak exercised holding steady and leaving
  matched clocks alone, but no live drop or repeat. The correction paths are
  proven by the simulations and the render unit tests only. Worth a soak on
  an output with its own clock (USB DAC, Bluetooth, HDMI) later.
- The step-4 cold start that settled 203 frames low (757) did not recur:
  this cold start settled at 960. An offset that size sits inside the 5 ms
  trigger and would be left alone by design.
- Other audio briefly played through the cable around 955-975 s of the soak:
  input peaked at -2.7 dBFS and the output was held at -1.0 dBFS with 0.9 dB
  of limiting, with no effect on the fill.
