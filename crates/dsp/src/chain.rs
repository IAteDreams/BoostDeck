//! The render thread's processing chain: gain -> soft clip -> limiter.
//!
//! One `Chain` per render stream, built before the stream starts. `process`
//! works in place on interleaved samples and allocates nothing.

use crate::gain::GainSmoother;
use crate::limiter::Limiter;
use crate::meter::{from_db, to_db};
use crate::softclip::SoftClip;
use crate::{GAIN_SMOOTHING_MS, LIMITER_CEILING_DBFS, LIMITER_RELEASE_MS, SOFT_CLIP_KNEE};

#[derive(Debug, Clone)]
pub struct Chain {
    channels: usize,
    gain: GainSmoother,
    soft_clip: SoftClip,
    limiter: Limiter,
}

/// What one buffer did, for the meters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Report {
    /// Largest output magnitude, linear.
    pub peak_out: f32,
    /// Deepest limiter gain reduction in the buffer, in dB, 0 or more.
    pub gain_reduction_db: f32,
}

impl Chain {
    /// Starts at unity gain, so a stream opened with a boost already set
    /// ramps up to it rather than starting loud.
    pub fn new(sample_rate: u32, channels: usize) -> Self {
        let ceiling = from_db(LIMITER_CEILING_DBFS);
        Self {
            channels,
            gain: GainSmoother::new(sample_rate, GAIN_SMOOTHING_MS, 1.0),
            soft_clip: SoftClip::new(SOFT_CLIP_KNEE * ceiling),
            limiter: Limiter::new(sample_rate, ceiling, LIMITER_RELEASE_MS),
        }
    }

    /// Processes whole interleaved frames in place, heading toward
    /// `target_gain_db`.
    ///
    /// A sample that is not finite becomes silence before anything else sees
    /// it: a NaN would slip past the limiter's peak check and reach the
    /// speakers.
    pub fn process(&mut self, samples: &mut [f32], target_gain_db: f32) -> Report {
        debug_assert_eq!(samples.len() % self.channels, 0, "partial frame");
        let target = from_db(target_gain_db);
        let mut peak_out = 0.0f32;
        let mut lowest_gain = 1.0f32;

        for frame in samples.chunks_exact_mut(self.channels) {
            let gain = self.gain.next(target);
            for sample in frame.iter_mut() {
                let clean = if sample.is_finite() { *sample } else { 0.0 };
                *sample = self.soft_clip.process(clean * gain);
            }
            lowest_gain = lowest_gain.min(self.limiter.process_frame(frame));
            for sample in frame.iter() {
                peak_out = peak_out.max(sample.abs());
            }
        }

        Report {
            peak_out,
            // `to_db(1.0)` is 0; the `max` turns a -0.0 into 0.0.
            gain_reduction_db: (-to_db(lowest_gain)).max(0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GAIN_MAX_DB_UNLOCKED, GAIN_MIN_DB};

    const RATE: u32 = 48_000;
    const CHANNELS: usize = 2;

    /// Deterministic noise without a dependency: xorshift32, mapped to
    /// [-amplitude, amplitude].
    struct Noise(u32);

    impl Noise {
        fn sample(&mut self, amplitude: f32) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0) * amplitude
        }
    }

    fn ceiling() -> f32 {
        from_db(LIMITER_CEILING_DBFS)
    }

    /// The README's test: for random input and any gain up to +24 dB, the
    /// output never exceeds the limiter ceiling. Covers full-scale and
    /// hotter-than-full-scale input, every gain in 1 dB steps, and the ramps
    /// between them.
    #[test]
    fn output_never_exceeds_the_ceiling() {
        let mut noise = Noise(0x2545_f491);
        let mut chain = Chain::new(RATE, CHANNELS);
        let mut buffer = [0.0f32; 480 * CHANNELS];

        let mut gain_db = GAIN_MIN_DB;
        while gain_db <= GAIN_MAX_DB_UNLOCKED {
            for amplitude in [0.1, 0.5, 1.0, 4.0] {
                for _ in 0..4 {
                    buffer.iter_mut().for_each(|s| *s = noise.sample(amplitude));
                    let report = chain.process(&mut buffer, gain_db);
                    let peak = crate::meter::peak(&buffer);
                    assert!(
                        peak <= ceiling(),
                        "{peak} over the ceiling at {gain_db} dB, amplitude {amplitude}"
                    );
                    assert_eq!(report.peak_out, peak);
                }
            }
            gain_db += 1.0;
        }
    }

    /// A gain jump straight from the bottom of the range to the top must be
    /// held too, while the smoother is mid-ramp.
    #[test]
    fn a_full_range_jump_never_exceeds_the_ceiling() {
        let mut noise = Noise(7);
        let mut chain = Chain::new(RATE, CHANNELS);
        let mut buffer = [0.0f32; 64 * CHANNELS];
        for round in 0..200 {
            let gain_db = if round % 50 < 25 {
                GAIN_MIN_DB
            } else {
                GAIN_MAX_DB_UNLOCKED
            };
            buffer.iter_mut().for_each(|s| *s = noise.sample(1.0));
            chain.process(&mut buffer, gain_db);
            assert!(crate::meter::peak(&buffer) <= ceiling());
        }
    }

    /// The README's test: at 0 dB, signal below the soft-clip knee passes
    /// unchanged - bit for bit, not approximately.
    #[test]
    fn at_0_db_signal_below_the_knee_passes_unchanged() {
        let knee = SOFT_CLIP_KNEE * ceiling();
        let mut noise = Noise(99);
        let mut chain = Chain::new(RATE, CHANNELS);
        let input: Vec<f32> = (0..4_800).map(|_| noise.sample(knee)).collect();
        let mut buffer = input.clone();

        let report = chain.process(&mut buffer, 0.0);

        assert_eq!(buffer, input);
        assert_eq!(report.gain_reduction_db, 0.0);
    }

    /// +6 dB doubles a quiet signal - the boost the spike is plugged in at -
    /// once the smoother has got there.
    #[test]
    fn plus_6_db_doubles_a_quiet_signal() {
        let mut chain = Chain::new(RATE, CHANNELS);
        // A second of audio: over 30 time constants, enough to snap on.
        let mut settle = vec![0.1f32; 48_000 * CHANNELS];
        chain.process(&mut settle, 6.0);

        let mut buffer = [0.1f32, -0.1];
        chain.process(&mut buffer, 6.0);
        let expected = 0.1 * from_db(6.0);
        assert!((buffer[0] - expected).abs() < 1e-6, "{}", buffer[0]);
        assert!((buffer[1] + expected).abs() < 1e-6, "{}", buffer[1]);
    }

    /// Driving into the ceiling shows up on the gain-reduction meter.
    #[test]
    fn heavy_boost_reports_gain_reduction() {
        let mut noise = Noise(3);
        let mut chain = Chain::new(RATE, CHANNELS);
        let mut buffer = [0.0f32; 9_600 * CHANNELS];
        buffer.iter_mut().for_each(|s| *s = noise.sample(0.9));

        let report = chain.process(&mut buffer, GAIN_MAX_DB_UNLOCKED);

        // At most the gap from full scale to the ceiling: the soft clip caps
        // the signal below full scale before the limiter sees it.
        let reduction = report.gain_reduction_db;
        assert!(
            reduction > 0.5 && reduction <= -LIMITER_CEILING_DBFS + 1e-3,
            "{report:?}"
        );
        assert!(report.peak_out <= ceiling());
    }

    #[test]
    fn non_finite_input_becomes_silence() {
        let mut chain = Chain::new(RATE, CHANNELS);
        let mut buffer = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.25];
        chain.process(&mut buffer, 0.0);
        assert_eq!(buffer, [0.0, 0.0, 0.0, 0.25]);
    }
}
