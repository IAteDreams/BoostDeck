//! Clock drift correction.
//!
//! VB-Cable and the output device run on separate clocks, so the ring buffer
//! slowly empties or fills. `DriftCorrector` tracks ring fill as a moving
//! average over `DRIFT_AVERAGE_MS` and, when it strays more than
//! `DRIFT_TRIGGER_MS` from the target, asks for one frame to be dropped or
//! repeated per `DRIFT_CORRECTION_PERIOD_FRAMES` frames until the average is
//! back at the target. That holds the fill within `DRIFT_TOLERANCE_MS`.
//! Must not allocate.
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

/// How far the fill may stray from target: the band drift correction holds.
pub const DRIFT_TOLERANCE_MS: u32 = 10;

/// How far the average fill may stray before correction starts: half the
/// tolerance. Drift does not creep in smoothly - capture delivers 10 ms
/// packets, so a fast clock shows up as one whole extra packet, a 10 ms step,
/// and a slow one as a missing packet. Triggering at the tolerance itself
/// meant one step left the average approaching the trigger and never passing
/// it, so nothing happened until a second step took the fill to 20 ms.
/// Triggering at half catches the first step, and pulls it back long before
/// the next one lands.
pub const DRIFT_TRIGGER_MS: u32 = DRIFT_TOLERANCE_MS / 2;

/// Correct by one frame per this many frames: 0.1%, inaudible on music.
pub const DRIFT_CORRECTION_PERIOD_FRAMES: u32 = 1_000;

/// Time constant of the fill average. Long enough to smooth over the
/// packet-sized steps in the fill - capture delivers 10 ms at a time, and
/// can land before or after render's pull on any tick - and short enough to
/// see real drift within a second or two.
pub const DRIFT_AVERAGE_MS: u32 = 1_000;

/// What to do to one frame of the next buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Correction {
    /// The ring is filling: capture's clock is fast. Pull one frame more than
    /// the device asked for and discard it.
    Drop,
    /// The ring is emptying: capture's clock is slow. Play one frame twice and
    /// pull one fewer.
    Repeat,
}

#[derive(Debug, Clone)]
pub struct DriftCorrector {
    target_frames: f32,
    trigger_frames: f32,
    average_frames_window: f32,
    /// Moving average of the fill left after each pull, in frames.
    average: f32,
    /// The correction in progress, if any.
    correcting: Option<Correction>,
    /// Frames played since the last correction while correcting.
    frames_since_correction: u32,
}

impl DriftCorrector {
    /// Starts with the average at the target, which is where priming leaves
    /// the ring.
    pub fn new(sample_rate: u32, target_frames: u32) -> Self {
        Self {
            target_frames: target_frames as f32,
            trigger_frames: (sample_rate * DRIFT_TRIGGER_MS / 1_000) as f32,
            average_frames_window: (sample_rate * DRIFT_AVERAGE_MS / 1_000) as f32,
            average: target_frames as f32,
            correcting: None,
            frames_since_correction: 0,
        }
    }

    /// Feeds the fill left in the ring after a pull of `frames`, and returns
    /// the correction to make in the next buffer, if one is due.
    ///
    /// Correction starts when the average strays past the trigger and stops
    /// only when it is back at the target, not merely back inside the
    /// trigger: stopping at the edge would leave it hovering there and
    /// starting again at every wobble.
    pub fn next(&mut self, fill_frames: u32, frames: u32) -> Option<Correction> {
        // A one-pole average stepped per buffer, weighted by the buffer's
        // length so the time constant holds whatever the device period.
        let weight = 1.0 - (-(frames as f32) / self.average_frames_window).exp();
        self.average += (fill_frames as f32 - self.average) * weight;
        let error = self.average - self.target_frames;

        self.correcting = match self.correcting {
            None if error > self.trigger_frames => Some(Correction::Drop),
            None if error < -self.trigger_frames => Some(Correction::Repeat),
            Some(Correction::Drop) if error <= 0.0 => None,
            Some(Correction::Repeat) if error >= 0.0 => None,
            unchanged => unchanged,
        };

        let correction = self.correcting?;
        self.frames_since_correction += frames;
        if self.frames_since_correction < DRIFT_CORRECTION_PERIOD_FRAMES {
            return None;
        }
        self.frames_since_correction -= DRIFT_CORRECTION_PERIOD_FRAMES;
        Some(correction)
    }

    /// The moving average of the fill, in frames.
    pub fn average_frames(&self) -> f32 {
        self.average
    }

    pub fn is_correcting(&self) -> bool {
        self.correcting.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const PACKET: u32 = 480;
    const TARGET: u32 = 960;
    const TOLERANCE: f32 = (RATE * DRIFT_TOLERANCE_MS / 1_000) as f32;

    #[derive(Debug, Default)]
    struct Run {
        lowest_fill: i64,
        highest_average: f32,
        lowest_average: f32,
        drops: u32,
        repeats: u32,
    }

    /// Simulates capture on a clock `ppm` parts per million fast (positive)
    /// or slow against render, for `seconds`, starting `offset_frames` off
    /// target. Capture delivers whole 480-frame packets as its clock
    /// accumulates them; render pulls 480 a tick, adjusted by the corrector.
    /// Panics on an underrun. Averages are only scored after `settle_secs`.
    fn simulate(ppm: f64, seconds: u32, offset_frames: i64, settle_secs: u32) -> Run {
        let mut corrector = DriftCorrector::new(RATE, TARGET);
        let mut fill = TARGET as i64 + offset_frames;
        let mut captured = 0.0f64;
        let mut pending = None;
        let mut run = Run {
            lowest_fill: i64::MAX,
            highest_average: f32::MIN,
            lowest_average: f32::MAX,
            ..Run::default()
        };

        let ticks = seconds * RATE / PACKET;
        let settle_ticks = settle_secs * RATE / PACKET;
        for tick in 0..ticks {
            captured += PACKET as f64 * (1.0 + ppm * 1e-6);
            while captured >= PACKET as f64 {
                fill += PACKET as i64;
                captured -= PACKET as f64;
            }

            let pull = match pending {
                Some(Correction::Drop) => {
                    run.drops += 1;
                    PACKET as i64 + 1
                }
                Some(Correction::Repeat) => {
                    run.repeats += 1;
                    PACKET as i64 - 1
                }
                None => PACKET as i64,
            };
            fill -= pull;
            assert!(fill >= 0, "underrun at tick {tick}, {ppm} ppm");

            pending = corrector.next(fill as u32, PACKET);
            if tick >= settle_ticks {
                run.lowest_fill = run.lowest_fill.min(fill);
                run.highest_average = run.highest_average.max(corrector.average_frames());
                run.lowest_average = run.lowest_average.min(corrector.average_frames());
            }
        }
        run
    }

    fn assert_held(case: &str, run: &Run) {
        let target = TARGET as f32;
        assert!(
            run.highest_average <= target + TOLERANCE && run.lowest_average >= target - TOLERANCE,
            "{case}: average ranged {:.0}..{:.0} frames, target {TARGET} +/- {TOLERANCE}",
            run.lowest_average,
            run.highest_average
        );
    }

    /// The README's test: simulated fast and slow clocks keep the fill within
    /// +/-10 ms of target. Two hours, as the step-5 "done when" asks of the
    /// real thing, at 100 ppm - well past what real clocks drift.
    #[test]
    fn fast_and_slow_clocks_hold_fill_within_tolerance_for_two_hours() {
        for ppm in [100.0, -100.0] {
            let run = simulate(ppm, 2 * 60 * 60, 0, 0);
            assert_held(&format!("{ppm} ppm"), &run);
            assert!(run.lowest_fill >= 0);
            // It had to work for it: a quarter of the drift is 1,440 frames
            // over two hours, and none of it may be left uncorrected.
            assert!(run.drops + run.repeats > 0, "{ppm} ppm: never corrected");
        }
    }

    /// Up to half the correction rate, drift is held; the corrector has room
    /// to spare at 0.1%.
    #[test]
    fn holds_drift_up_to_half_the_correction_rate() {
        for ppm in [500.0, -500.0] {
            let run = simulate(ppm, 10 * 60, 0, 0);
            assert_held(&format!("{ppm} ppm"), &run);
        }
    }

    /// A fill that starts well off target - a bad prime - is brought back and
    /// then held.
    #[test]
    fn brings_an_offset_back_to_target() {
        for offset in [700, -700] {
            // 700 frames at 1 per 1,000 is ~15 s of correction.
            let run = simulate(0.0, 120, offset, 30);
            assert_held(&format!("offset {offset}"), &run);
        }
    }

    /// Matched clocks need nothing: not one drop or repeat.
    #[test]
    fn matched_clocks_are_left_alone() {
        let run = simulate(0.0, 10 * 60, 0, 0);
        assert_eq!((run.drops, run.repeats), (0, 0));
    }

    /// Inside the trigger the average is left where it is: the corrector
    /// only answers drift, not every offset. 203 frames is the offset a cold
    /// start once settled at.
    #[test]
    fn an_offset_inside_the_trigger_is_left_alone() {
        let run = simulate(0.0, 60, -203, 0);
        assert_eq!((run.drops, run.repeats), (0, 0));
    }

    /// Corrections come no faster than one per 1,000 frames.
    #[test]
    fn corrects_at_one_frame_per_thousand() {
        let mut corrector = DriftCorrector::new(RATE, TARGET);
        let mut corrections = 0;
        // Far over target for a full second of 480-frame buffers.
        for _ in 0..100 {
            if corrector.next(TARGET * 10, PACKET).is_some() {
                corrections += 1;
            }
        }
        // 48,000 frames at one per 1,000, less the time the average took to
        // cross the tolerance.
        assert!((40..=48).contains(&corrections), "{corrections}");
    }
}
