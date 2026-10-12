//! Soft clipper.
//!
//! Rounds the signal off above `SOFT_CLIP_KNEE` of the ceiling so the limiter
//! has less work to do and hard edges never reach the output. Must not
//! allocate.
//!
//! Below the knee a sample passes unchanged. Above it the magnitude follows a
//! tanh curve that leaves the knee at slope 1 - no corner - and flattens
//! toward full scale, which it never reaches. Holding the result to the
//! ceiling is the limiter's job, not this one's.

/// Where the curve flattens out: full scale.
const LIMIT: f32 = 1.0;

#[derive(Debug, Clone)]
pub struct SoftClip {
    knee: f32,
    /// Room between the knee and full scale.
    span: f32,
}

impl SoftClip {
    /// `knee` is linear, between 0 and full scale.
    pub fn new(knee: f32) -> Self {
        debug_assert!(knee > 0.0 && knee < LIMIT);
        Self {
            knee,
            span: LIMIT - knee,
        }
    }

    #[inline]
    pub fn process(&self, sample: f32) -> f32 {
        let magnitude = sample.abs();
        if magnitude <= self.knee {
            return sample;
        }
        let rounded = self.knee + self.span * ((magnitude - self.knee) / self.span).tanh();
        rounded.copysign(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNEE: f32 = 0.6;

    #[test]
    fn below_the_knee_passes_unchanged() {
        let clip = SoftClip::new(KNEE);
        for sample in [0.0, 0.1, -0.3, 0.59, -0.6, KNEE] {
            assert_eq!(clip.process(sample), sample);
        }
    }

    #[test]
    fn never_reaches_full_scale_however_hot_the_input() {
        let clip = SoftClip::new(KNEE);
        for sample in [0.61, 0.9, 1.0, 2.0, 16.0, 1e6, f32::MAX] {
            let out = clip.process(sample);
            assert!(out > KNEE && out <= LIMIT, "{sample} -> {out}");
            assert_eq!(clip.process(-sample), -out, "symmetry at {sample}");
        }
    }

    /// Louder in, louder (or equal) out: the curve never folds back.
    #[test]
    fn is_monotonic() {
        let clip = SoftClip::new(KNEE);
        let mut previous = 0.0;
        for step in 0..=4_000 {
            let out = clip.process(step as f32 / 1_000.0);
            assert!(out >= previous, "folded back at {}", step as f32 / 1_000.0);
            previous = out;
        }
    }

    /// No corner at the knee: just above it the curve still has slope ~1.
    #[test]
    fn leaves_the_knee_without_a_corner() {
        let clip = SoftClip::new(KNEE);
        let delta = 1e-3;
        let slope = (clip.process(KNEE + delta) - KNEE) / delta;
        assert!((slope - 1.0).abs() < 0.01, "slope {slope}");
    }
}
