//! Peak limiter, always on.
//!
//! Holds the output at or below `LIMITER_CEILING_DBFS` with an
//! `LIMITER_RELEASE_MS` release, and reports its gain for the gain-reduction
//! meter. Must not allocate.
//!
//! Attack is instant: a frame that would cross the ceiling is turned down by
//! exactly enough, on that frame, so nothing gets past while the gain moves.
//! Release is a one-pole back toward unity. Instant attack distorts sharp
//! transients slightly; the soft clip ahead of it takes the edge off.
//!
//! A 2-5 ms look-ahead buffer is a later improvement, not MVP.

#[derive(Debug, Clone)]
pub struct Limiter {
    /// Linear ceiling.
    ceiling: f32,
    /// Gain currently applied, 0 to 1.
    gain: f32,
    release_coeff: f32,
}

impl Limiter {
    /// `ceiling` is linear; `release_ms` is the release time constant.
    pub fn new(sample_rate: u32, ceiling: f32, release_ms: f32) -> Self {
        let release_frames = release_ms / 1_000.0 * sample_rate as f32;
        Self {
            ceiling,
            gain: 1.0,
            release_coeff: 1.0 - (-1.0 / release_frames).exp(),
        }
    }

    /// Limits one frame, all channels by the same gain so the stereo image
    /// holds. Returns the gain applied.
    #[inline]
    pub fn process_frame(&mut self, frame: &mut [f32]) -> f32 {
        let peak = frame.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
        let wanted = if peak > self.ceiling {
            self.ceiling / peak
        } else {
            1.0
        };

        if wanted < self.gain {
            self.gain = wanted;
        } else {
            self.gain += (wanted - self.gain) * self.release_coeff;
        }

        for sample in frame {
            // `gain <= wanted` already holds the peak to the ceiling; the
            // clamp only catches the last bit of float rounding in the
            // multiply, so the ceiling is a hard guarantee.
            *sample = (*sample * self.gain).clamp(-self.ceiling, self.ceiling);
        }
        self.gain
    }

    pub fn gain(&self) -> f32 {
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const CEILING: f32 = 0.5;
    const RELEASE_MS: f32 = 80.0;

    /// The gain alone - before the rounding clamp - must hold the ceiling, so
    /// the clamp is never what the guarantee rests on.
    #[test]
    fn the_gain_alone_holds_the_ceiling() {
        let mut limiter = Limiter::new(RATE, CEILING, RELEASE_MS);
        for peak in [0.4, 0.6, 1.0, 3.0, 0.7, 50.0, 0.2] {
            let mut frame = [peak, -peak * 0.5];
            limiter.process_frame(&mut frame);
            let unclamped = peak * limiter.gain();
            assert!(unclamped <= CEILING * (1.0 + 1e-6), "{peak} -> {unclamped}");
        }
    }

    #[test]
    fn leaves_quiet_frames_alone() {
        let mut limiter = Limiter::new(RATE, CEILING, RELEASE_MS);
        let mut frame = [0.3, -0.49];
        assert_eq!(limiter.process_frame(&mut frame), 1.0);
        assert_eq!(frame, [0.3, -0.49]);
    }

    /// Both channels move together: limiting one loud side must not shift
    /// the balance.
    #[test]
    fn keeps_the_stereo_balance() {
        let mut limiter = Limiter::new(RATE, CEILING, RELEASE_MS);
        let mut frame = [1.0, 0.25];
        limiter.process_frame(&mut frame);
        assert_eq!(frame, [0.5, 0.125]);
    }

    /// After a loud burst the gain comes back up over the release time
    /// rather than snapping - and does come back.
    #[test]
    fn releases_gradually_after_a_burst() {
        let mut limiter = Limiter::new(RATE, CEILING, RELEASE_MS);
        limiter.process_frame(&mut [2.0, 2.0]);
        assert_eq!(limiter.gain(), 0.25);

        let mut quiet = [0.1, 0.1];
        limiter.process_frame(&mut quiet);
        assert!(limiter.gain() < 0.26, "snapped back to {}", limiter.gain());

        let release_frames = (RELEASE_MS / 1_000.0 * RATE as f32) as usize;
        for _ in 0..5 * release_frames {
            limiter.process_frame(&mut [0.1, 0.1]);
        }
        assert!(limiter.gain() > 0.99, "stuck at {}", limiter.gain());
    }
}
