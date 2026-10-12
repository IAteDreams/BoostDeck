//! Preset and settings structs.
//!
//! Step 9: the MVP fields only — id, name, gain, output device id. Later
//! features add fields with `#[serde(default)]` so old files keep parsing.
//! The limiter has no on/off switch in the MVP; it is always on.
//!
//! Built-in presets on first run: Default (0 dB), Quiet videos (+6 dB),
//! Max (+12 dB).
