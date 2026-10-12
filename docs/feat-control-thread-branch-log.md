# Branch log: feat/control-thread

## Merged 2026-10-12

Merged into `main` as a fast-forward. The engine now runs itself behind one
`Engine` handle: a control thread routes and plays on command, and rides out
a lost output, a sample-rate change, a failed stream and a default moved by
the user, without the program being restarted (coding-order step 7).

### Commits

| Commit | Subject |
| --- | --- |
| `d8459b5` | Add the control thread: commands, events, device notifications, rebuild with backoff |
| `df18f10` | Resume when Windows hands the default back to the cable while paused |
| `b7048ef` | Rebuild when the output's sample rate changes, and name a lost output |
| (this commit) | Document feat/control-thread for the main merge |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +1 -1 |
| `crates/engine/examples/common/mod.rs` | added, +141 -0 |
| `crates/engine/examples/control.rs` | added, +239 -0 |
| `crates/engine/examples/devices.rs` | modified, +17 -0 |
| `crates/engine/examples/passthrough.rs` | modified, +12 -134 |
| `crates/engine/src/control.rs` | modified, +799 -3 |
| `crates/engine/src/devices.rs` | modified, +8 -0 |
| `crates/engine/src/routing.rs` | modified, +19 -0 |
| `crates/engine/src/rt.rs` | modified, +7 -0 |
| `docs/feat-control-thread-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 78 tests (app 3, dsp 25 + 2 no-alloc, engine 48) |
| `control 15` | routed, played at 960 frames, 0 underruns; on stop restored Speakers (Realtek) and cleared the record |
| Default moved to ARZOPA mid-run (`devices --set-default`) | paused at 6.2 s, left ARZOPA as the default, record cleared |
| `control 20 --fail-at 7` (faked render failure) | Reconnecting attempt 1, playing again 0.3 s later, 0 underruns |
| Scripted: default to ARZOPA, then to `CABLE In 16ch`, then to CABLE Input | re-routed and played to ARZOPA; paused with no answer on the virtual cable; resumed on the same route when the default came back to CABLE Input |
| Bluetooth headphones (Monster Airmars), run by the user | picked by hand in Sound settings: re-routed to them at once; switched off: fell back to Speakers (Realtek), 0 underruns, 1 discontinuity at the switch |
| Sample rate 48000 -> 44100 -> 48000 Hz on the Realtek, run by the user | rebuilt to 44100 Hz at 49.9 s and back to 48000 Hz at 69.9 s; 0 underruns, 1 discontinuity per rebuild; fill back at 960 after the second |
| Sleep and resume, run by the user | audio carried on with no restart; the log shows no rebuild |

### Notes

- Step 7 is ticked in the README.
- Found by the user's headphone test: after a pause, Bluetooth headphones
  switching off made Windows pick CABLE Input as the next default, leaving
  the PC silent with the record already cleared. A paused route is now kept,
  and playback resumes when the default lands back on CABLE Input.
- Found by the user's sample-rate test: streams opened with autoconvert
  survive a format change, Windows resampling behind them, so nothing failed
  and nothing rebuilt. The control thread now listens for
  `PKEY_AudioEngine_DeviceFormat` and rebuilds when the output's rate really
  differs. The cable's own rate is left to autoconvert.
- Sleep on this laptop (most likely Modern Standby) did not break the
  streams, so the sleep path through a failed stream was only exercised with
  `--fail-at`.
- Windows does not make newly connected Bluetooth headphones the default by
  itself; picking them in Sound settings is the trigger. The app will offer
  "boost that one instead?"; the `control` example answers yes on the user's
  behalf, and also to "your output is back, switch to it?".
- At 44100 Hz the fill swung about one packet (713..1196, band 441..1323)
  where 48000 Hz sits flat at 960: the cable stays at 48 kHz and is
  resampled, so its packets no longer line up with the render period.
- The examples now share their helpers (`examples/common`), and the routing
  record is the `RoutingJournal` trait the app will implement over its
  settings in step 8. `devices --set-default` sets the Windows default from
  the command line, for tests.
