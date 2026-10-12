# Branch log: feat/routing

## Merged 2026-10-12

Merged into `main` as a fast-forward. BoostDeck can now make CABLE Input the
Windows default output and reliably put the user's own device back - on a
normal exit, on Ctrl+C, on a panic, and on the next launch after being killed
outright - so a crash never leaves the PC silent (coding-order step 6).

### Commits

| Commit | Subject |
| --- | --- |
| `b09e6f2` | Add default-device routing: switch to CABLE Input, restore on exit, panic and next launch |
| (this commit) | Document feat/routing for the main merge |

### Files

| File | Change |
| --- | --- |
| `Cargo.lock` | modified, +3 -0 |
| `README.md` | modified, +1 -1 |
| `crates/app/src/main.rs` | modified, +10 -7 |
| `crates/app/src/settings.rs` | modified, +118 -5 |
| `crates/app/src/state.rs` | modified, +82 -3 |
| `crates/app/src/ui.rs` | modified, +9 -1 |
| `crates/engine/Cargo.toml` | modified, +8 -0 |
| `crates/engine/examples/passthrough.rs` | modified, +165 -5 |
| `crates/engine/src/policy_config.rs` | modified, +79 -4 |
| `crates/engine/src/routing.rs` | modified, +288 -4 |
| `docs/feat-routing-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 73 tests (app 3, dsp 25 + 2 no-alloc, engine 43) |
| `passthrough 10 --route`, starting with CABLE Input as default (set by hand in step 3) | no usable original, fell back to Speakers (Realtek); default was CABLE Input during the run with `routing_active: true` recorded; restored to Realtek on exit and the record cleared; 0 underruns |
| Step 6 "done when": `passthrough 600 --route`, killed with `taskkill /F`, then `boostdeck.exe` launched | after the kill the default stayed on CABLE Input with the record set; the app logged "routing_active was left set: the last run did not clean up", restored Speakers (Realtek) and cleared the record; closing the app ran `on_exit` |
| Ctrl+C on `passthrough 600 --route`, run by the user | "Interrupted.", both threads finished cleanly, 0 underruns, default restored to Speakers (Realtek), record cleared |
| Communications role | stayed on ARZOPA (NVIDIA) through every test |

### Notes

- Step 6 is ticked in the README.
- `IPolicyConfig` is declared with `windows_core::interface` against
  `windows` 0.62, as the README asks; unused methods keep untyped pointer
  arguments, only `SetDefaultEndpoint` is called. `BOOL` lives in
  `windows_core` in 0.62, not `Win32::Foundation`.
- Settings stay the app's: the engine takes the saved original and a
  `record` closure, so it never touches `settings.json`. The `passthrough`
  example writes the same two fields through a small JSON journal (other
  fields carried through) so a killed example run is recovered by the real
  app.
- Restores run on a short-lived MTA thread (`restore_on_own_thread`) rather
  than on eframe's main thread, whose COM apartment belongs to winit.
- The panic hook leaves `routing_active` set, so the launch after a panic
  still reports that the last run ended badly. It was not exercised live:
  nothing can panic on purpose yet. It shares `set_default_output` with the
  paths that were.
- Follow-up: the app has no Ctrl+C handler of its own. Once it routes (steps
  7-8), add one like the example's, or Ctrl+C in a debug console skips
  `on_exit` and leaves recovery to the next launch.
- The Windows default output is now Speakers (Realtek), not the hand-set
  CABLE Input from step 3: `--route` switches it when needed, and a cable
  default with nothing reading it is silence.
