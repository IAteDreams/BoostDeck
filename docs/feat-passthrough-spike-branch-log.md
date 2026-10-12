# Branch log: feat/passthrough-spike

## Merged 2026-10-12

Merged into `main` as a fast-forward. BoostDeck now has a working audio path:
audio played into VB-Cable is captured, carried through a lock-free ring and
played on the real speakers at unity gain, on MMCSS-registered real-time
threads, with a `passthrough` example that runs it and reports ring fill,
peaks and glitch counts (coding-order step 3).

### Commits

| Commit | Subject |
| --- | --- |
| `ef8b7ca` | Add the capture -> ring -> render pass-through spike |
| `9d0a69d` | Prime the ring by settling, so it holds its target after each pull |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +10 -1 |
| `crates/engine/examples/passthrough.rs` | added, +204 -0 |
| `crates/engine/src/capture.rs` | modified, +269 -7 |
| `crates/engine/src/render.rs` | modified, +475 -7 |
| `crates/engine/src/rt.rs` | modified, +272 -9 |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 25 tests |
| 10-minute soak, `passthrough -- 600`, looping `Ring01.wav` into CABLE Input, Realtek speakers at 48 kHz | 0 underruns, 0 overruns, 0 discontinuities in playback; input and output both -15.2 dBFS throughout; both threads finished cleanly |
| Five 15 s runs with the same sound | ring fill 960 frames, 0 underruns in each |

### Notes

- The first spike run had 1 underrun and a ring fill stuck at 480 frames
  against a 960-frame target. Two causes: priming paid for the render
  device's initial whole-buffer request (1,056 frames) out of the target, and
  priming on a single tick locked in a lost capture packet at random. `9d0a69d`
  primes by playing the ring into the void until it has been in step for 10
  device periods, and reports fill after the pull.
- The soak used a looping system sound, not the video the README's "done when"
  names. Counters say the path held; listening for crackles with a real video
  is still worth doing.
- Follow-up: in the soak, priming took about 5 s instead of ~100 ms (the first
  5 s window shows output silent and 57,600 startup overruns, cleared by 10 s).
  Requiring 10 *consecutive* in-step ticks is brittle when capture and render
  keep swapping order on the engine tick.
- Follow-up: steady-state margin is one capture packet (480 frames), not the
  full target. About a fifth of the soak's 5 s readings showed 480, recovering
  to 960, as the tick order flipped. That was enough for zero underruns over
  10 minutes; step 5's drift correction is the place to steer it properly.
