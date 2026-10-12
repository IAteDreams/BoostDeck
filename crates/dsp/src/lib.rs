//! Gain, soft clip, limiter, meters and drift correction.
//!
//! This crate is pure Rust with no Windows code and no dependencies, so it
//! builds and tests on any machine. The processing path must allocate nothing:
//! no `Vec` growth, no `Box`, no `String`, no `println!`.
//!
//! Coding-order step 4 filled in the processing chain; step 5 fills in drift
//! correction.

pub mod chain;
pub mod drift;
pub mod gain;
pub mod limiter;
pub mod meter;
pub mod softclip;

pub use chain::{Chain, Report};

/// Limiter ceiling in dBFS. Fixed in the MVP; there is no UI switch.
pub const LIMITER_CEILING_DBFS: f32 = -1.0;

/// Gain smoothing time. Every gain change, preset loads included, goes
/// through the smoother so the level never jumps.
pub const GAIN_SMOOTHING_MS: f32 = 30.0;

/// Soft-clip knee, as a fraction of the limiter ceiling.
pub const SOFT_CLIP_KNEE: f32 = 0.70;

/// Limiter release time.
pub const LIMITER_RELEASE_MS: f32 = 80.0;

/// Lowest gain the slider offers.
pub const GAIN_MIN_DB: f32 = -60.0;

/// Highest gain the slider offers by default.
pub const GAIN_MAX_DB: f32 = 12.0;

/// Highest gain after the user unlocks it in settings, behind a warning
/// dialog about hearing and speaker damage.
pub const GAIN_MAX_DB_UNLOCKED: f32 = 24.0;
