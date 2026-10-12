# Branch log: feat/dsp-chain

## Merged 2026-10-12

Merged into `main` as a fast-forward. BoostDeck can now boost: every buffer
the render thread plays goes through a smoothed gain, a soft clip and an
always-on limiter at -1 dBFS, with the gain set through `Shared` and the
`passthrough` spike taking `--gain-db` (coding-order step 4).

### Commits

| Commit | Subject |
| --- | --- |
| `4099f74` | Add the DSP chain and plug it into the render thread |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +2 -1 |
| `crates/dsp/src/chain.rs` | added, +205 -0 |
| `crates/dsp/src/gain.rs` | modified, +86 -3 |
| `crates/dsp/src/lib.rs` | modified, +5 -1 |
| `crates/dsp/src/limiter.rs` | modified, +118 -3 |
| `crates/dsp/src/meter.rs` | modified, +41 -3 |
| `crates/dsp/src/softclip.rs` | modified, +84 -2 |
| `crates/dsp/tests/no_alloc.rs` | added, +93 -0 |
| `crates/engine/examples/passthrough.rs` | modified, +49 -10 |
| `crates/engine/src/render.rs` | modified, +114 -60 |
| `crates/engine/src/rt.rs` | modified, +52 -0 |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 50 tests (dsp 19 + 2 no-alloc, engine 29) |
| Mutation checks | removing the limiter from the chain fails the ceiling tests (0.91 against 0.89); a `Box` in `process` fails the no-alloc test (200 allocations) |
| `--gain-db` validation | +13, a missing value and a non-number are refused |
| Live A/B, 15 s each, looping `Ring01.wav`, Realtek at 48 kHz | 0 dB: -15.2 dBFS in, -15.2 out; +6 dB: -15.2 in, -9.2 out; 0 underruns |
| 10-minute soak at +6 dB from 75 s idle | 0 underruns, 0 overruns, 0 discontinuities; -15.2 in, -9.2 out in all 120 readings; 0.0 dB gain reduction |

### Notes

- Step 4 is not ticked in the README: its "done when" includes "the boost is
  audible", which needs a person listening. The meters show a steady +6.0 dB.
- With the README's gain -> soft clip -> limiter order, the soft clip caps the
  signal below full scale before the limiter sees it, so limiter gain
  reduction never exceeds ~1 dB and heavy boosts are shaped mostly by tanh
  saturation. Matches the spec; revisit if the limiter should do the gain
  riding.
- The limiter is always on, so at 0 dB audio above -4.1 dBFS (the soft-clip
  knee) is now rounded off. Step 3's unity-gain tests were replaced.
- The gain smoother stalled short of its target in f32 (0.5000429 for 0.5); it
  now snaps on once a step makes no progress, which keeps 0 dB bit-exact.
- Deviations from the README architecture: the no-alloc test uses a counting
  allocator instead of `assert_no_alloc` (dsp stays dependency-free; README
  updated); `Shared` has no limiter-ceiling atomic since the ceiling is fixed
  in the MVP; meters are input peak, output peak and gain reduction rather
  than pre- and post-limiter peaks.
- Follow-up, not from this branch: the soak's cold start settled the ring low
  point at 757 frames rather than 960 (readings 757/1237, offsets that are not
  whole 480-frame packets), steady for 10 minutes. Warm runs at 0 and +6 dB
  held 960, so it is not the gain. Likely an odd-sized capture delivery after
  priming measured the low point; step 5's drift correction should absorb it.
