//! Smoothed gain stage.
//!
//! Step 4: a one-pole smoother over the target gain read from `Shared`, so a
//! slider move or preset load ramps over `GAIN_SMOOTHING_MS` instead of
//! jumping. Must not allocate.
