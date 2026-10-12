# Branch log: fix/priming-settle

## Merged 2026-10-12

Merged into `main` as a fast-forward. The pass-through now starts playing
about 200 ms after launch whether the speakers were idle or not, where a cold
start used to stay silent for seconds, and the ring sits at its 960-frame
target from the start instead of sometimes settling a packet short.

### Commits

| Commit | Subject |
| --- | --- |
| `c3a8d33` | Prime on the ring's low point, so a cold start plays in ~200 ms |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +10 -7 |
| `crates/engine/src/render.rs` | modified, +173 -76 |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 26 tests |
| Priming time, temporary timer, after 75 s idle | 198 ms cold; 218 ms on three warm starts |
| Cold starts after 75 s idle, 10 s each, plus warm follow-ups | 6 runs, all playing in the first 5 s window, fill 960, 0 underruns |
| 10-minute soak from 75 s idle, `passthrough -- 600`, looping `Ring01.wav` into CABLE Input, Realtek speakers at 48 kHz | 0 underruns, 0 overruns, 0 discontinuities; all 120 fill readings 960 frames; input and output -15.2 dBFS in every window |

### Notes

- Cause: a cable fresh from idle delivers capture in bursts - two packets on
  one tick, none on the next. The previous priming trimmed each burst back to
  the target, so the next empty tick always failed its 10-in-a-row check.
  Reproduced from per-tick logs: never primed within 10 s.
- Priming now pulls as playback will for a 200 ms window and sets the lowest
  post-pull fill to the target. It starts with one request of headroom; with
  only one, warm starts needed a second window (~430 ms). The tests'
  one-window deadline fails if that headroom is removed.
- Resolves both follow-ups in `docs/feat-passthrough-spike-branch-log.md`: the
  slow start, and the 480-frame fill readings, which were startup settling on
  a low point - none appeared in this soak.
- The fill figure is a snapshot every 5 s, not the minimum between snapshots.
  A packet missed during playback, rather than priming, still costs margin
  until step 5's drift correction exists.
