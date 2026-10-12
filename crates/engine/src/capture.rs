//! Capture thread: reads CABLE Output into the ring buffer.
//!
//! Step 3. Real-time thread — see the crate docs for what it may not do.
//! Opens the stream in shared, event-driven mode as 32-bit float stereo at the
//! real device's sample rate with `autoconvert: true`, so Windows resamples if
//! the cable runs at another rate.
//!
//! The `wasapi` crate's own `loopback` example has this same two-thread
//! capture-plus-render shape and is the best starting point.
