//! Clock drift correction.
//!
//! VB-Cable and the output device run on separate clocks, so the ring buffer
//! slowly empties or fills. Step 5: track ring fill as a moving average over
//! about 1 second and, when it strays more than `DRIFT_TOLERANCE_MS` from the
//! target, drop or repeat one frame per `DRIFT_CORRECTION_PERIOD_FRAMES`
//! frames until it comes back. Must not allocate.
//!
//! If pure tones later show artifacts, replace the drop/repeat with an
//! adaptive resampler such as `rubato` nudged by a few parts per million.

/// Target amount of audio waiting in the ring.
pub const RING_TARGET_MS: u32 = 20;

/// Range the user may set the ring target to. Lower means less delay and more
/// risk of crackles.
pub const RING_TARGET_MS_RANGE: (u32, u32) = (10, 60);

/// Ring capacity, in seconds of audio. Large enough never to fill in normal use.
pub const RING_CAPACITY_SECS: f32 = 0.5;

/// How far the average fill may stray from target before correcting.
pub const DRIFT_TOLERANCE_MS: u32 = 10;

/// Correct by one frame per this many frames: 0.1%, inaudible on music.
pub const DRIFT_CORRECTION_PERIOD_FRAMES: u32 = 1_000;
