# Branch log: feat/presets

## Merged 2026-10-13

Merged into `main` as a fast-forward. BoostDeck now keeps named presets of
boost and output in a versioned `presets.json` that survives a damaged file,
and the window gains a presets panel to load, save, rename, duplicate and
delete them (coding-order step 9).

### Commits

| Commit | Subject |
| --- | --- |
| `fda5c91` | Add presets: versioned store with backup recovery, and the presets panel |
| (this commit) | Document feat/presets for the main merge |

### Files

| File | Change |
| --- | --- |
| `Cargo.lock` | modified, +1 -0 |
| `README.md` | modified, +1 -1 |
| `crates/app/Cargo.toml` | modified, +1 -0 |
| `crates/app/src/state.rs` | modified, +201 -5 |
| `crates/app/src/ui.rs` | modified, +205 -4 |
| `crates/presets/src/lib.rs` | modified, +6 -4 |
| `crates/presets/src/migrate.rs` | modified, +145 -4 |
| `crates/presets/src/model.rs` | modified, +62 -7 |
| `crates/presets/src/store.rs` | modified, +571 -7 |
| `docs/feat-presets-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 95 tests (app 6, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| README tests | save/load round trip, name rules, last-preset protection, corrupt file to `.bak` then to built-ins, version 0 fixture to version 1: all covered |
| Mutation check | not moving the broken file aside fails both corrupt-file tests |
| Window, driven through UI Automation with keystrokes, checked by screenshot | load Max (+12 dB); slider to 9 dB marked "Max •" and Save wrote 9.0; Save as new "Movies" made it active with the Speakers output; Rename to "Films"; Duplicate to "Films copy"; Delete of "Max copy" through its confirmation, leaving no active preset |
| Launch with a +9 dB preset active | started at +6 dB, marked modified |
| Step 9 "done when": `presets.json` cut off mid-file by hand, app relaunched | moved aside as `presets.broken-1791820956.json`, all six presets restored from `.bak` with a notice in the window; the backup left intact |

### Notes

- Step 9 is ticked in the README.
- Deviation from the build guide: no `LimiterSettings` on a preset. The
  limiter is fixed in the MVP and nothing would read them; a later feature
  can add them with `#[serde(default)]`.
- Version 0 had no real prior release, so it is defined in `migrate.rs` as
  a bare array of presets with no ids and a plain `output_device_id`, to give
  the README's migration test something to migrate.
- A preset with no output (the built-ins) plays wherever boost is playing:
  the output never marks it modified, and Save keeps it output-free. "Save
  as new" captures the current output.
- The active preset is applied at launch with its gain capped at +6 dB. That
  is the core of step 10's safe start; the "Restore +X dB?" offer is still
  step 10's.
- The backup is one save behind, so a recovery brings back the state before
  the last change (the deleted "Max copy" returned in the test).
- Found while testing the window: the rename field opened without keyboard
  focus. Name fields now take focus with their text selected.
- Windows UI Automation's set-value does not reach egui text fields; the
  window test typed names with real keystrokes instead.
- The test presets were deleted afterwards, so the next launch is a first
  run with the three built-ins.
