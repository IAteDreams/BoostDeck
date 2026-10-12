//! Smoothed gain stage.
//!
//! A one-pole smoother over the target gain read from `Shared`, so a slider
//! move or preset load ramps over `GAIN_SMOOTHING_MS` instead of jumping.
//! Must not allocate.

/// One-pole smoother over linear gain, stepped once per frame so every channel
/// of a frame gets the same gain.
#[derive(Debug, Clone)]
pub struct GainSmoother {
    current: f32,
    coeff: f32,
}

impl GainSmoother {
    /// `time_ms` is the time constant: the gain covers 63% of a step in one,
    /// and 99% in five.
    pub fn new(sample_rate: u32, time_ms: f32, initial: f32) -> Self {
        let time_constant_frames = time_ms / 1_000.0 * sample_rate as f32;
        Self {
            current: initial,
            coeff: 1.0 - (-1.0 / time_constant_frames).exp(),
        }
    }

    /// Advances one frame toward `target` and returns the gain for it.
    #[inline]
    pub fn next(&mut self, target: f32) -> f32 {
        let stepped = self.current + (target - self.current) * self.coeff;
        // A one-pole only ever approaches its target, and in f32 it stops
        // short: once a step is under half an ulp of the gain it rounds to no
        // change (0.5 stalled at 0.5000429, -0.0007 dB). Snapping when a step
        // makes no progress makes "at target" exact, which is what lets 0 dB
        // pass samples through bit for bit.
        self.current = if stepped == self.current {
            target
        } else {
            stepped
        };
        self.current
    }

    pub fn current(&self) -> f32 {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GAIN_SMOOTHING_MS;

    const RATE: u32 = 48_000;

    fn frames_for(time_ms: f32) -> usize {
        (time_ms / 1_000.0 * RATE as f32).round() as usize
    }

    /// The README's test: 99% of a new target within five time constants.
    #[test]
    fn reaches_99_percent_within_five_time_constants() {
        let mut smoother = GainSmoother::new(RATE, GAIN_SMOOTHING_MS, 1.0);
        let target = 4.0;
        for _ in 0..frames_for(5.0 * GAIN_SMOOTHING_MS) {
            smoother.next(target);
        }
        let covered = (smoother.current() - 1.0) / (target - 1.0);
        assert!(covered >= 0.99, "covered {covered}");
    }

    /// No sudden jumps: a step never lands in one frame.
    #[test]
    fn a_step_is_ramped_not_jumped() {
        let mut smoother = GainSmoother::new(RATE, GAIN_SMOOTHING_MS, 1.0);
        let first = smoother.next(4.0);
        assert!(first > 1.0 && first < 1.01, "first frame at {first}");
    }

    /// Smoothing works downwards too, and settles exactly on the target.
    #[test]
    fn settles_exactly_on_the_target() {
        let mut smoother = GainSmoother::new(RATE, GAIN_SMOOTHING_MS, 4.0);
        for _ in 0..frames_for(30.0 * GAIN_SMOOTHING_MS) {
            smoother.next(0.5);
        }
        assert_eq!(smoother.current(), 0.5);
    }
}
