//! Real-time thread support: the `Shared` atomics and MMCSS registration.
//!
//! Coding-order step 3. `Shared` is the only thing the UI and the audio
//! threads touch in common, and it holds nothing but atomics: the real-time
//! threads must never block, so there is no lock here and never will be.
//!
//! The UI writes parameters and reads meters; the render thread does the
//! opposite. Problems are reported by setting `error`, which the control
//! thread polls - a real-time thread may not log, allocate or send.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use anyhow::{Context as _, Result};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
};
use windows::core::w;

/// Samples are 32-bit float throughout.
pub const BYTES_PER_SAMPLE: usize = 4;

/// Stereo only in the MVP. More channels are a later feature.
pub const CHANNELS: u16 = 2;

/// Why an audio thread gave up. Stored in `Shared` as a `u32` because an
/// atomic is the only way a real-time thread can report anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtError {
    None,
    /// The capture client failed, or the device went away.
    Capture,
    /// The render client failed, or the device went away.
    Render,
    /// The device stopped signalling its event. Usually a removed or
    /// reconfigured endpoint.
    Timeout,
}

impl RtError {
    fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Capture => 1,
            Self::Render => 2,
            Self::Timeout => 3,
        }
    }

    fn from_code(code: u32) -> Self {
        match code {
            1 => Self::Capture,
            2 => Self::Render,
            3 => Self::Timeout,
            _ => Self::None,
        }
    }
}

/// State shared between the UI, the control thread and the two real-time
/// threads. Atomics only.
#[derive(Debug, Default)]
pub struct Shared {
    stop: AtomicBool,
    error: AtomicU32,

    // Peak meters hold `f32::to_bits` of a non-negative magnitude. For values
    // at or above zero the IEEE-754 bit pattern sorts in the same order as the
    // number itself, so `fetch_max` over the bits is a correct max over the
    // floats. That only holds while the stored value is non-negative, which is
    // why these are fed absolute values.
    peak_in: AtomicU32,
    peak_out: AtomicU32,

    /// Times the render thread wanted samples and the ring was empty. Each one
    /// is an audible gap, so this is the number to watch.
    underruns: AtomicU64,
    /// Times the capture thread had a sample and the ring was full.
    overruns: AtomicU64,
    /// Buffers Windows marked discontinuous, meaning it knows audio was lost
    /// before it ever reached us.
    discontinuities: AtomicU64,

    /// How much audio is waiting in the ring, in frames. Drift correction
    /// steers this toward its target in step 5.
    ring_fill_frames: AtomicU32,
}

impl Shared {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask both real-time threads to finish at their next buffer.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn should_stop(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }

    /// Called from a real-time thread on the way out. Keeps the first error
    /// rather than the last: that is the one that explains the rest.
    pub fn set_error(&self, error: RtError) {
        let _ = self.error.compare_exchange(
            RtError::None.code(),
            error.code(),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }

    pub fn error(&self) -> RtError {
        RtError::from_code(self.error.load(Ordering::Relaxed))
    }

    pub fn clear_error(&self) {
        self.error.store(RtError::None.code(), Ordering::Relaxed);
    }

    /// `peak` is taken as a magnitude; see the note on the fields.
    pub fn report_peak_in(&self, peak: f32) {
        self.peak_in
            .fetch_max(peak.abs().to_bits(), Ordering::Relaxed);
    }

    pub fn report_peak_out(&self, peak: f32) {
        self.peak_out
            .fetch_max(peak.abs().to_bits(), Ordering::Relaxed);
    }

    /// Reads the input peak and resets it, so each read covers only the period
    /// since the last one.
    pub fn take_peak_in(&self) -> f32 {
        f32::from_bits(self.peak_in.swap(0, Ordering::Relaxed))
    }

    pub fn take_peak_out(&self) -> f32 {
        f32::from_bits(self.peak_out.swap(0, Ordering::Relaxed))
    }

    pub fn count_underrun(&self) {
        self.underruns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count_overrun(&self) {
        self.overruns.fetch_add(1, Ordering::Relaxed);
    }

    pub fn count_discontinuity(&self) {
        self.discontinuities.fetch_add(1, Ordering::Relaxed);
    }

    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn overruns(&self) -> u64 {
        self.overruns.load(Ordering::Relaxed)
    }

    pub fn discontinuities(&self) -> u64 {
        self.discontinuities.load(Ordering::Relaxed)
    }

    /// Clears the glitch counters. Called once when the render thread primes,
    /// so the totals cover playback rather than the initial ring fill.
    pub fn reset_counters(&self) {
        self.underruns.store(0, Ordering::Relaxed);
        self.overruns.store(0, Ordering::Relaxed);
        self.discontinuities.store(0, Ordering::Relaxed);
    }

    pub fn set_ring_fill_frames(&self, frames: u32) {
        self.ring_fill_frames.store(frames, Ordering::Relaxed);
    }

    pub fn ring_fill_frames(&self) -> u32 {
        self.ring_fill_frames.load(Ordering::Relaxed)
    }
}

/// Registers the calling thread with MMCSS under the "Pro Audio" task, so
/// Windows schedules it like other professional audio work, and reverts that
/// registration on drop.
///
/// Create this on the thread it applies to: the registration is per-thread,
/// which is why the handle is deliberately not `Send`.
pub struct ProAudioPriority {
    handle: HANDLE,
}

impl ProAudioPriority {
    pub fn register() -> Result<Self> {
        let mut task_index = 0u32;
        // SAFETY: a static task name and a live out-pointer. The returned
        // handle is reverted exactly once, in Drop, on this same thread.
        let handle = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task_index) }
            .context("registering the thread with MMCSS as Pro Audio")?;
        Ok(Self { handle })
    }
}

impl Drop for ProAudioPriority {
    fn drop(&mut self) {
        // SAFETY: `handle` came from AvSetMmThreadCharacteristicsW on this
        // thread and is reverted exactly once.
        let _ = unsafe { AvRevertMmThreadCharacteristics(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_meters_keep_the_loudest_and_reset_on_read() {
        let shared = Shared::new();
        shared.report_peak_in(0.25);
        shared.report_peak_in(0.80);
        shared.report_peak_in(0.10);

        assert_eq!(shared.take_peak_in(), 0.80);
        // Reset, so the next window starts clean.
        assert_eq!(shared.take_peak_in(), 0.0);
    }

    /// Samples are signed, so the meter has to see magnitude for the
    /// fetch_max-over-bits trick to hold.
    #[test]
    fn peak_meters_use_magnitude() {
        let shared = Shared::new();
        shared.report_peak_out(-0.9);
        assert_eq!(shared.take_peak_out(), 0.9);
    }

    #[test]
    fn the_first_error_is_the_one_kept() {
        let shared = Shared::new();
        assert_eq!(shared.error(), RtError::None);

        shared.set_error(RtError::Capture);
        shared.set_error(RtError::Render);
        assert_eq!(shared.error(), RtError::Capture);

        shared.clear_error();
        assert_eq!(shared.error(), RtError::None);
        shared.set_error(RtError::Render);
        assert_eq!(shared.error(), RtError::Render);
    }

    #[test]
    fn stop_is_sticky() {
        let shared = Shared::new();
        assert!(!shared.should_stop());
        shared.request_stop();
        assert!(shared.should_stop());
    }

    #[test]
    fn counters_accumulate() {
        let shared = Shared::new();
        shared.count_underrun();
        shared.count_underrun();
        shared.count_overrun();
        shared.count_discontinuity();

        assert_eq!(shared.underruns(), 2);
        assert_eq!(shared.overruns(), 1);
        assert_eq!(shared.discontinuities(), 1);
    }
}
