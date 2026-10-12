//! Windows-only audio engine.
//!
//! Four threads run the audio path (see README "Architecture rules"):
//!
//! | Thread | Owns | Talks through | Real-time? |
//! | --- | --- | --- | --- |
//! | UI (eframe main) | App state, presets, settings | `EngineCommand` out, `EngineEvent` in | No |
//! | Control | Device enumerator, notifications, routing, stream lifetime | Channels to UI; `Arc<Shared>` to RT threads | No |
//! | Capture | CABLE Output capture client | `rtrb::Producer<f32>` | Yes |
//! | Render | Real-device render client, DSP state | `rtrb::Consumer<f32>`, `Arc<Shared>` | Yes |
//!
//! The two real-time threads must not allocate after startup, lock, log, or do
//! file or channel I/O. Every buffer is allocated before the stream starts, and
//! problems are reported by setting an atomic flag the control thread polls.
//!
//! Filled in by coding-order steps 2, 3, 6 and 7.

#![cfg(windows)]

pub mod capture;
pub mod console;
pub mod control;
pub mod devices;
pub mod policy_config;
pub mod render;
pub mod routing;
pub mod rt;

/// Friendly-name fragment identifying VB-Cable's render endpoint, the device
/// BoostDeck makes the Windows default while boost is on.
pub const CABLE_INPUT_NAME: &str = "CABLE Input";

/// Friendly-name fragment identifying VB-Cable's capture endpoint, which
/// BoostDeck reads the mixed system audio from.
pub const CABLE_OUTPUT_NAME: &str = "CABLE Output";

/// Backoff between stream-rebuild attempts. After the last one fails, the
/// control thread restores the original default device and reports an error
/// rather than leave the user in silence.
pub const REBUILD_BACKOFF_MS: [u64; 5] = [250, 500, 1_000, 2_000, 4_000];
