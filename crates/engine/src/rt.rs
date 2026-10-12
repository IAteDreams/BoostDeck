//! Real-time thread support.
//!
//! Step 3: register the capture and render threads with MMCSS so Windows
//! schedules them like other pro-audio work, and hold the `Shared` struct of
//! atomics that the UI and the audio threads exchange values through.
//!
//! `Shared` holds only atomics: target gain (`AtomicU32` of `f32::to_bits`),
//! limiter ceiling, a `stop` flag, an `error` flag, pre- and post-limiter peak
//! meters, limiter gain reduction, and an underrun counter. The UI writes
//! parameters and reads meters; the render thread does the opposite. Slider
//! moves never go through a channel.
