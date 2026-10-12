//! Render thread: ring buffer to the real output device, through the DSP.
//!
//! Step 3 for the pass-through, step 4 once the DSP is plugged in. Real-time
//! thread — see the crate docs for what it may not do.
//!
//! Per buffer: read the target gain from `Shared`, run gain -> soft clip ->
//! limiter, store the peak meters and limiter gain reduction back into
//! `Shared`, and let `dsp::drift` nudge the ring back toward its target fill.
//! Outputs silence until the ring first reaches the target.
