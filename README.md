# BoostDeck

Boost every app's sound past 100% on Windows, with an always-on limiter so nothing clips.

> **Status: pre-implementation.** The MVP is being built; no crates are committed yet.
> See [Coding order](#coding-order) for what lands in which step.

BoostDeck works in **cable mode**: it routes all Windows audio through a virtual cable
(VB-Cable), processes it, and plays the result on your real speakers or headphones. No
audio driver, no APO and no code signing are needed — the cost is about 35–80 ms of
added delay.

## How cable mode works

1. BoostDeck sets **CABLE Input** (VB-Cable) as the Windows default playback device, so
   every shared-mode app plays into it.
2. VB-Cable passes that audio to its recording side, **CABLE Output**.
3. BoostDeck's capture thread reads CABLE Output with WASAPI and pushes samples into a
   lock-free ring buffer.
4. BoostDeck's render thread pulls from the ring, applies gain → soft clip → limiter, and
   writes to the real speakers or headphones you picked.
5. On exit, BoostDeck restores your original default device.

Only the render thread changes the sound; the control and UI threads never touch audio
samples.

**In the MVP:** master boost slider, always-on limiter, output device picker, level
meters, VB-Cable setup check, default-device switch and restore, and named presets (save,
load, rename, duplicate, delete).

**After the MVP:** APO/driver modes, per-app volumes, EQ, compressor, tray icon, hotkeys,
auto-start and 5.1/7.1 support.

## Requirements

- Windows 10 22H2 or Windows 11
- [VB-CABLE](https://vb-audio.com/Cable/), installed by you — BoostDeck does not bundle it
- A real output device (speakers or headphones) to play the processed audio on

**Cannot be boosted in cable mode:** apps that use WASAPI exclusive mode or ASIO (they
bypass the Windows mixer), and some DRM-protected streams.

## Setting up VB-Cable

1. Download VB-CABLE from [vb-audio.com/Cable](https://vb-audio.com/Cable/), run the
   64-bit setup **as administrator**, then reboot.
2. Open the classic Sound control panel (`mmsys.cpl`). Note your real speakers' format
   under Properties → Advanced (usually 48000 Hz).
3. Set **CABLE Input** (Playback tab) and **CABLE Output** (Recording tab) to that same
   sample rate. Matching rates avoid an extra resampling step.
4. On CABLE Output → Listen tab, make sure **Listen to this device** is off, or you will
   hear everything twice.
5. On CABLE Input → Advanced, untick **Allow applications to take exclusive control**, so
   no app can grab the cable away from the mixer.
6. Optional, once BoostDeck is working: lower the cable's max latency in VB-Audio's
   control panel to cut delay, and raise it again if you hear crackles.

## Safety

> Boosted audio with heavy limiting can damage small laptop speakers and your hearing if
> played loud for long periods. BoostDeck shows this notice once on first launch.

If you are running an early development build, use headphones at low system volume for
every test — a bug in the gain path can produce full-scale noise — and keep a way to
restore audio by hand (set your speakers back as the default output in Sound settings).

| Rule | How it works |
| --- | --- |
| Limiter always on | No UI switch; ceiling fixed at −1 dBFS in the MVP |
| Boost cap +12 dB | Slider and presets clamp to `max_boost_db` (12) |
| Unlock to +24 dB | Settings checkbox behind a warning dialog about hearing and speaker damage |
| No sudden jumps | Every gain change, including preset loads, goes through the 30 ms smoother |
| Safe start | If the saved gain is above +6 dB, start at +6 dB and offer "Restore +X dB?" |
| Panic button | A visible **0 dB** button that sets gain to 0 dB at once |
| Start at 0 dB on new output | Switching output device resets gain to 0 dB until you raise it |
| Never leave silence | Any unrecoverable error restores the original default device before reporting |

## Known limitations

- About 35–80 ms of added delay; fine for music and video, noticeable in fast games.
- Apps using WASAPI exclusive mode or ASIO bypass the cable and are not boosted.
- Some DRM-protected streams may play silent or unboosted.
- Requires VB-Cable installed by the user.
- While boost is on, the Windows volume slider controls CABLE Input, not the speakers.
- Stereo only in the MVP; 5.1 and 7.1 are a later feature.

---

## Development

### Toolchain

1. Install Visual Studio Build Tools with the **Desktop development with C++** workload
   and a current Windows SDK (Rust's MSVC target links against them).
2. Install Rust with `rustup`, target `x86_64-pc-windows-msvc`. Use Rust **1.95 or
   newer**: eframe 0.36 requires it.
3. Optional but useful: `cargo install cargo-watch` for fast rebuilds.

You also need VB-Cable set up as above, with its rate matching your speakers'.

### Workspace layout

A Cargo workspace of four crates, so the DSP and preset code stay pure Rust and testable
on any machine, and all Windows code lives in one place.

```
boostdeck/
├─ Cargo.toml              # workspace root
├─ rust-toolchain.toml     # channel = "stable"; the 1.95 floor is in Cargo.toml
├─ crates/
│  ├─ app/                 # binary: eframe GUI, wiring, settings
│  │  └─ src/ main.rs, ui.rs, state.rs, settings.rs
│  ├─ engine/              # Windows-only audio engine
│  │  └─ src/ lib.rs, control.rs, capture.rs, render.rs,
│  │          devices.rs, routing.rs, policy_config.rs, rt.rs
│  ├─ dsp/                 # pure Rust, no allocation in process calls
│  │  └─ src/ lib.rs, gain.rs, softclip.rs, limiter.rs, meter.rs, drift.rs
│  └─ presets/             # data model + JSON storage
│     └─ src/ lib.rs, model.rs, store.rs, migrate.rs
└─ assets/                 # app icon
```

### Dependencies

Versions as of October 2026 — run `cargo update` when you start.

| Crate | Version | Used in | For |
| --- | --- | --- | --- |
| `wasapi` | 0.26 | engine | Capture, render, device list, notifications |
| `windows` / `windows-core` | 0.62 | engine | `IPolicyConfig`, MMCSS thread priority |
| `rtrb` | 0.4 | engine | Lock-free sample ring between threads |
| `crossbeam-channel` | 0.5 | engine, app | Commands and events between UI and engine |
| `eframe` | 0.36 | app | GUI |
| `serde` + `serde_json` | 1 | presets, app | Preset and settings files |
| `directories` | 6 | presets, app | `%APPDATA%` paths |
| `uuid` | 1 (`v4`, `serde`) | presets | Preset ids |
| `log` + `simplelog` | 0.4 / 0.12 | app, engine, presets | File logging; never in `dsp`, which must not log on the audio threads |
| `anyhow` / `thiserror` | 1 / 2 | all | Errors |

Keep `windows` pinned to 0.62 so it matches what `wasapi` 0.26 uses; two versions in one
build compile but their types do not mix.

`windows` features needed by the engine: `Win32_Foundation`, `Win32_Media_Audio`,
`Win32_System_Com`, `Win32_System_Threading`.

### Build and test

```sh
cargo build                          # debug build
cargo clippy                         # lints
cargo run -p app                     # launch the GUI
cargo run -p engine --example devices # list endpoints, check VB-Cable
cargo run -p engine --example passthrough -- 600  # 10-min audio soak test
cargo test                           # pure-Rust tests, any machine
cargo test --features device-tests   # also hits real devices; needs VB-Cable
cargo build --release                # release build
```

**Automated (`cargo test`, any machine)**

- DSP: for random input and any gain up to +24 dB, output never exceeds the limiter ceiling
- DSP: at 0 dB, signal below the soft-clip knee passes unchanged
- DSP: smoother reaches 99% of a new target within 5× its time constant
- DSP: `process` makes no heap allocations (a counting allocator in `dsp/tests/no_alloc.rs`,
  in place of `assert_no_alloc`, so `dsp` keeps no dependencies)
- Drift: simulated fast and slow clocks keep fill within ±10 ms of target
- Presets: save/load round trip, name rules, last-preset protection
- Presets: corrupt file falls back to `.bak`, then to built-ins
- Presets: a schema version 0 fixture migrates to version 1

**Device tests (`--features device-tests`, a PC with VB-Cable)**

- Capture from CABLE Output and render to CABLE Input loop together for 60 s without errors

**Manual, before each release**

- Music, a video and a game all play boosted through the cable
- Unplug and replug headphones while playing
- Change the speakers' sample rate in Sound settings while playing
- Sleep and resume the PC
- Change the Windows default output by hand while boost is on
- Kill BoostDeck in Task Manager, relaunch, confirm the default is restored
- Run for 8 hours; underrun count stays at 0 and delay does not grow
- Click test: measure delay from CABLE Input to speakers and note it in the release notes
- Uninstall VB-Cable; BoostDeck shows the setup screen and audio still works normally

### Architecture rules

These are the invariants the build guide fixes. Do not break them; see the guide for how
each one is implemented.

**Four threads**

| Thread | Owns | Talks to others through | Real-time? |
| --- | --- | --- | --- |
| UI (eframe main) | App state, presets, settings | `EngineCommand` out, `EngineEvent` in (crossbeam channels) | No |
| Control | `DeviceEnumerator`, device notifications, default-device switch/restore, starting and stopping streams | Channels to UI; `Arc<Shared>` to RT threads | No |
| Capture | CABLE Output capture client | `rtrb::Producer<f32>` | Yes |
| Render | Real-device render client, DSP state | `rtrb::Consumer<f32>`, `Arc<Shared>` atomics | Yes |

**Real-time threads** — no heap allocation after startup, no locks, no logging, no file or
channel I/O. Allocate every buffer before `start_stream()`. Report problems by setting an
atomic flag the control thread polls. Both RT threads register with MMCSS.

**`Shared` holds only atomics:** target gain (`AtomicU32` storing `f32::to_bits`), limiter
ceiling, a `stop` flag, an `error` flag, peak meters (pre and post limiter), limiter gain
reduction, and an underrun counter. The UI writes parameters and reads meters; the render
thread does the opposite. Slider moves never go through a channel.

**Stream format** — open both streams in shared, event-driven mode as 32-bit float stereo
at the real device's sample rate, with `autoconvert: true`.

**DSP defaults**

| Parameter | Value |
| --- | --- |
| Gain range | −60 dB to +12 dB (unlock to +24 dB in settings) |
| Gain smoothing | 30 ms |
| Soft-clip knee | 70% of the ceiling |
| Limiter ceiling | −1 dBFS |
| Limiter release | 80 ms |

**Buffering and clock drift** — ring capacity 0.5 s of stereo samples; target fill 20 ms
(a setting from 10 to 60 ms); the render thread outputs silence until the ring first
reaches target. VB-Cable and the speakers run on separate clocks, so track ring fill as a
~1 s moving average and, when it strays more than 5 ms from target, drop or repeat one
frame per 1,000 frames until it is back at target. That holds the fill within ±10 ms: capture
delivers 10 ms packets, so drift arrives as whole 10 ms steps, and triggering at 10 ms itself
would let one step sit at the trigger unanswered until a second took the fill to 20 ms.

Priming keeps the speakers silent for ~200 ms while it plays the ring into the void, pulling
exactly as playback will, and watches the lowest fill left after a pull. It then puts that
low point at the target: excess is dropped, and a shortfall is waited out by holding pulls
back. Watching the low point rather than the fill at one instant is what makes this
independent of timing: capture and render wake on the same engine tick in either order, and
a cable fresh from idle delivers two packets on one tick and none on the next. The cable
also hands over a backlog while the render stream is still opening, and because capture and
render then both run at the device rate, a ring that starts full *stays* full — a stable
equilibrium at the worst possible latency. Discarding it before the first sample is played
is inaudible; ongoing drift is corrected a frame at a time instead.

**Latency budget at 48 kHz** (estimates — measure your own)

| Stage | Typical delay |
| --- | --- |
| VB-Cable internal buffer | 10–40 ms, depends on its max-latency setting |
| Capture buffer (device period) | 3–10 ms |
| Ring target fill | 20 ms |
| Render buffer (device period) | 3–10 ms |
| **Total** | **about 35–80 ms** |

**Default device routing** — Windows has no public API for changing the default device.
Declare the undocumented `IPolicyConfig` COM interface yourself against `windows` 0.62
(method order must match exactly); the `com-policy-config` crate wraps it but depends on
`windows` 0.61. Save the original device id to `settings.json` with `routing_active: true`
**before** switching. Leave the **Communications** role alone. Restore the original from a
panic hook (`std::panic::set_hook`) and from eframe's `on_exit`; on launch, if
`routing_active` is still true, the last run did not clean up — restore immediately and
say why. Never offer a VB-Audio virtual cable as BoostDeck's own output: CABLE Input
would feed the cable back into itself, and the extra variants an install ships
(`CABLE In 16ch`, `Cable A`) send audio somewhere nothing is listening.

**Device changes** go to the control thread only; the RT threads raise a flag and exit.
Windows calls device notifications on its own threads, so each callback just posts a
message. Rebuild streams with backoff at 250 ms, 500 ms, 1 s, 2 s and 4 s; if all five
fail, restore the original default and show an error rather than leave the user in
silence.

**Presets** — one versioned `presets.json` under
`directories::ProjectDirs::from("com", "BoostDeck", "BoostDeck")`. Save atomically: write
`presets.json.tmp`, flush, copy the old file to `presets.json.bak`, then rename the temp
over `presets.json`. On a parse failure fall back to `.bak`; if that fails too, rename the
broken file to `presets.broken-<timestamp>.json`, start from the built-ins (Default 0 dB,
Quiet videos +6 dB, Max +12 dB) and tell the user.

### Coding order

Build the audio path first without any GUI, prove it is stable for hours, then wrap it in
routing, the UI and presets.

- [x] **1. Skeleton.** Workspace, four crates, `cargo run -p app` opens an empty eframe window. *Done when:* `cargo build` and `cargo clippy` pass.
- [x] **2. Device list.** A CLI in `engine/examples/` listing render and capture devices. *Done when:* it prints both CABLE ids and your speakers' id.
- [x] **3. Pass-through spike.** CABLE Input set as default by hand; capture → ring → render at unity gain, no GUI. *Done when:* a video plays through for 10 minutes with no crackles.
- [ ] **4. DSP crate.** Gain, soft clip, limiter, meters, unit tests; plugged into the spike at +6 dB. *Done when:* tests prove output never exceeds the ceiling and the boost is audible.
- [x] **5. Drift correction.** Ring-fill tracking and drop/repeat correction, fill logged every 5 s. *Done when:* a 2-hour run holds fill within ±10 ms of target with zero underruns.
- [x] **6. Routing.** `IPolicyConfig` switch, save original, restore on exit, panic hook, recovery on next launch. *Done when:* killing the process in Task Manager and relaunching restores your speakers as default.
- [x] **7. Control thread.** Commands, events, device notifications, rebuild with backoff. *Done when:* unplugging headphones, changing the sample rate, and sleep/resume all recover without a restart.
- [ ] **8. GUI.** Status line, on/off, slider, meters, output picker, setup screen, wired to the engine. *Done when:* everything in steps 3–7 works from the window.
- [ ] **9. Presets.** `presets` crate, storage, panel, built-ins. *Done when:* create, rename, duplicate, delete and load all work, and a hand-corrupted `presets.json` recovers from the backup.
- [ ] **10. Safety rules.** Caps, unlock dialog, safe start, panic button, first-run notice. *Done when:* every row of the [Safety](#safety) table is checked by hand.
- [ ] **11. Package.** Unsigned release build and installer. *Done when:* it installs and runs on a second PC with VB-Cable.

### Packaging

The MVP ships unsigned as a portable zip plus an MSI.

- `app/src/main.rs` carries `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`,
  so release builds open no console window while debug builds keep one for logs.
- In the workspace `Cargo.toml`: `[profile.release] opt-level = 3`, `lto = "thin"`,
  `codegen-units = 1`. Keep the default `panic = "unwind"`.
- Embed an app icon with a resource crate such as `embed-resource`.
- Write the log file to `%LOCALAPPDATA%\BoostDeck\logs`, keeping the last 5 files.
- Portable: zip the `.exe` with a README covering the VB-Cable steps.
- MSI: `cargo install cargo-wix`, then `cargo wix init` and `cargo wix` inside the app
  crate. Install the WiX Toolset version your cargo-wix release expects.
- Do not bundle VB-Cable; link to vb-audio.com and let the setup screen guide the user.

Without signing, users see "Windows protected your PC" on first run (More info → Run
anyway). On PCs with Smart App Control turned on the unsigned installer is blocked; those
users need the portable zip, or a signed build later.

## Reference

The authoritative spec is the
[BoostDeck Cable Mode — Rust Build Guide](https://claude.ai/code/artifact/5aea8c0c-ad1e-4eaf-875a-5c16ec1cfd7c),
which carries the full code sketches for every section above.

- [wasapi crate 0.26 documentation](https://docs.rs/wasapi/latest/wasapi/)
- wasapi [`loopback` example](https://docs.rs/crate/wasapi/latest/source/examples/loopback.rs) and [`devices` example](https://docs.rs/crate/wasapi/latest/source/examples/devices.rs) — the `loopback` example has the same two-thread shape and is the best thing to copy from
- [com-policy-config 0.6 source](https://docs.rs/crate/com-policy-config/latest/source/src/lib.rs) (`IPolicyConfig` layout and GUIDs)
- [VB-Audio VB-CABLE](https://vb-audio.com/Cable/)

## License

MIT — see [LICENSE](LICENSE).
