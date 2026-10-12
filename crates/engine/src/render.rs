//! Render thread: ring buffer to the real output device.
//!
//! Coding-order steps 3 and 4. This is a real-time thread, so once the stream
//! is running it must not allocate, lock, log, or touch a file or a channel.
//!
//! Each device buffer is pulled from the ring and run through `dsp::Chain` -
//! gain toward the target in `Shared`, soft clip, limiter - before it is
//! written out. The limiter is always on, so even at 0 dB anything above the
//! soft-clip knee is rounded off and nothing passes the ceiling.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use rtrb::Consumer;
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::rt::{BYTES_PER_SAMPLE, CHANNELS, ProAudioPriority, RtError, Shared};

/// How long to wait for the device to signal a buffer before treating the
/// stream as dead.
const EVENT_TIMEOUT_MS: u32 = 2_000;

/// Spare room in the byte staging queue, as a multiple of one device buffer.
const STAGING_BUFFERS: usize = 8;

/// Device periods priming watches the ring for before playback starts: about
/// 200 ms at a 10 ms period. Long enough to span several capture deliveries,
/// which a cable fresh from idle makes in bursts of two packets and then none.
const SETTLE_TICKS: u32 = 20;

pub struct RenderConfig {
    /// Endpoint id of the real output device. Never a virtual cable: see
    /// `devices::output_candidates`.
    pub device_id: String,
    pub sample_rate: u32,
    /// How much audio to have waiting in the ring before the first sample is
    /// played. Until the ring reaches this, the device is fed silence.
    pub target_fill_frames: u32,
}

/// Runs until `Shared::request_stop` is set or the stream fails.
///
/// Call this as the entire body of a dedicated thread: it initialises COM for
/// the thread and registers it with MMCSS.
pub fn run(config: RenderConfig, consumer: Consumer<f32>, shared: Arc<Shared>) -> Result<()> {
    let result = run_inner(config, consumer, &shared);
    if let Err(error) = &result {
        shared.set_error(match error.downcast_ref::<Timeout>() {
            Some(_) => RtError::Timeout,
            None => RtError::Render,
        });
    }
    result
}

/// Marker so a stalled device is distinguishable from an outright failure.
#[derive(Debug)]
struct Timeout;

impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the render device stopped signalling its event")
    }
}

impl std::error::Error for Timeout {}

fn run_inner(config: RenderConfig, mut consumer: Consumer<f32>, shared: &Shared) -> Result<()> {
    initialize_mta().ok().context("initialising COM (MTA)")?;
    let _priority = ProAudioPriority::register()?;

    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;
    let device = enumerator
        .get_device(&config.device_id)
        .with_context(|| format!("opening render device {}", config.device_id))?;
    let mut audio_client = device
        .get_iaudioclient()
        .context("getting the render audio client")?;

    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        config.sample_rate as usize,
        CHANNELS as usize,
        None,
    );
    let block_align = format.get_blockalign() as usize;

    let (_default_period, min_period) = audio_client
        .get_device_period()
        .context("reading the render device period")?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_period,
    };
    audio_client
        .initialize_client(&format, &Direction::Render, &mode)
        .context("initialising the render client")?;

    let event = audio_client
        .set_get_eventhandle()
        .context("getting the render event handle")?;
    let buffer_frames = audio_client
        .get_buffer_size()
        .context("reading the render buffer size")? as usize;
    let render_client = audio_client
        .get_audiorenderclient()
        .context("getting the render client")?;

    let mut output = Output::new(config.sample_rate, buffer_frames, block_align);

    audio_client
        .start_stream()
        .context("starting the render stream")?;

    let outcome = render_loop(
        &audio_client,
        &render_client,
        &event,
        &mut output,
        &mut consumer,
        shared,
        config.target_fill_frames,
    );

    let _ = audio_client.stop_stream();
    outcome
}

fn render_loop(
    audio_client: &wasapi::AudioClient,
    render_client: &wasapi::AudioRenderClient,
    event: &wasapi::Handle,
    output: &mut Output,
    consumer: &mut Consumer<f32>,
    shared: &Shared,
    target_fill_frames: u32,
) -> Result<()> {
    // Silence goes out until the ring has settled at its target; see
    // `Priming`. Starting to pull from a nearly empty ring would underrun
    // immediately.
    let mut primed = false;
    let mut priming = Priming::default();
    let buffer_frames = audio_client
        .get_buffer_size()
        .context("reading the render buffer size")? as usize;

    while !shared.should_stop() {
        let frames = audio_client
            .get_available_space_in_frames()
            .context("reading the available render space")? as usize;

        if primed {
            process_to_staging(output, consumer, shared, frames);
        } else {
            if priming.tick(consumer, frames, buffer_frames, target_fill_frames as usize) {
                primed = true;
                // Everything counted so far belongs to filling the ring, not
                // to playback. Reset so the totals describe the steady state.
                shared.reset_counters();
            }
            fill_with_silence(&mut output.staging, frames);
        }

        // Measured after the pull: this is the low-water mark, the margin left
        // for the next capture packet, and the number the target describes.
        shared.set_ring_fill_frames((consumer.slots() / CHANNELS as usize) as u32);

        render_client
            .write_to_device_from_deque(frames, &mut output.staging, None)
            .context("writing to the render device")?;

        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            return Err(Timeout.into());
        }
    }
    Ok(())
}

/// Brings the ring to its target before the first sample is played.
///
/// The speakers get silence while the ring is played into the void - pulled
/// exactly as playback will pull it - and the lowest fill left after a pull is
/// watched for `SETTLE_TICKS` device periods. One adjustment then puts that
/// low point at the target: any excess is dropped, or, if the ring came up
/// short, pulls are held back until it has gained the difference and the
/// window is watched again.
///
/// Watching the low point is what makes this independent of how capture's
/// deliveries line up with render's pulls. Capture and render wake on the same
/// engine tick in either order, and a cable fresh from idle delivers two
/// packets on one tick and none on the next. Earlier versions primed on the
/// fill at a single tick: one locked in a packet's worth of shortfall at
/// random, and one trimmed every burst back to the target, so the next empty
/// tick always failed and a cold start could stay silent for seconds.
#[derive(Debug, Default)]
struct Priming {
    /// The startup backlog has been dropped and a window is being watched.
    started: bool,
    /// Device periods watched in the current window.
    ticks: u32,
    /// Lowest fill left after a pull in the current window, in frames.
    low_frames: usize,
    /// Frames the ring still has to gain, by skipping pulls, after a window
    /// came up short.
    hold_frames: usize,
}

impl Priming {
    /// One device event of `frames` requested. Returns true once the ring is
    /// at its target and playback should start.
    fn tick(
        &mut self,
        consumer: &mut Consumer<f32>,
        frames: usize,
        buffer_frames: usize,
        target_frames: usize,
    ) -> bool {
        // The device's first requests fill its whole buffer (1,056 frames on a
        // Realtek at 48 kHz). They are not a period and say nothing about
        // steady state.
        if frames == 0 || frames >= buffer_frames {
            return false;
        }

        if !self.started {
            // A request of headroom past what the first pull needs, so a window
            // in which capture runs a packet behind still ends with excess to
            // drop rather than a shortfall to wait out: holding costs a second
            // window, while excess is trimmed at the end for free.
            let start_frames = target_frames + 2 * frames;
            if queued_frames(consumer) < start_frames {
                return false;
            }
            // Drop the startup backlog: see `trim_to_target`.
            trim_to_target(consumer, start_frames as u32);
            self.watch_again();
        }

        if self.hold_frames > 0 {
            self.hold_frames = self.hold_frames.saturating_sub(frames);
            if self.hold_frames == 0 {
                self.watch_again();
            }
            return false;
        }

        drop_frames(consumer, frames);
        self.low_frames = self.low_frames.min(queued_frames(consumer));
        self.ticks += 1;
        if self.ticks < SETTLE_TICKS {
            return false;
        }

        if self.low_frames >= target_frames {
            drop_frames(consumer, self.low_frames - target_frames);
            return true;
        }
        self.hold_frames = target_frames - self.low_frames;
        false
    }

    fn watch_again(&mut self) {
        self.started = true;
        self.ticks = 0;
        self.low_frames = usize::MAX;
    }
}

fn queued_frames(consumer: &Consumer<f32>) -> usize {
    consumer.slots() / CHANNELS as usize
}

/// Drops up to `frames` frames from the front of the ring.
fn drop_frames(consumer: &mut Consumer<f32>, frames: usize) {
    let keep_frames = queued_frames(consumer).saturating_sub(frames);
    trim_to_target(consumer, keep_frames as u32);
}

/// Drops everything in the ring past `keep_frames`.
///
/// At startup the ring can be far past target: the cable hands over a backlog
/// while the render stream is still being opened, and capture and render then
/// both run at the device rate, so a ring that starts full *stays* full. That
/// is a stable equilibrium at the worst possible latency - a nearly full
/// half-second ring measured 488 ms of delay before this was added.
///
/// Discarding is audible in principle, but this only runs while priming, before
/// the first sample is played. Ongoing drift is corrected a frame at a time in
/// step 5, which is inaudible; this is deliberately not that.
fn trim_to_target(consumer: &mut Consumer<f32>, keep_frames: u32) {
    let target_samples = keep_frames as usize * CHANNELS as usize;
    while consumer.slots() > target_samples {
        if consumer.pop().is_err() {
            break;
        }
    }
}

/// Everything the render thread writes through on its way to the device.
/// Allocated before the stream starts and never grown afterwards.
struct Output {
    /// Bytes queued for the device.
    staging: VecDeque<u8>,
    /// One device buffer of interleaved samples, processed in place.
    samples: Vec<f32>,
    chain: dsp::Chain,
}

impl Output {
    fn new(sample_rate: u32, buffer_frames: usize, block_align: usize) -> Self {
        Self {
            staging: VecDeque::with_capacity(block_align * buffer_frames * STAGING_BUFFERS),
            samples: vec![0.0; buffer_frames * CHANNELS as usize],
            chain: dsp::Chain::new(sample_rate, CHANNELS as usize),
        }
    }
}

/// Fills one device buffer from the ring, padding with silence if the ring
/// runs dry, runs it through the DSP chain at the target gain in `Shared`,
/// queues it for the device and reports the meters.
fn process_to_staging(
    output: &mut Output,
    consumer: &mut Consumer<f32>,
    shared: &Shared,
    frames: usize,
) {
    // The device never asks for more than its buffer; `min` keeps a bad
    // request from panicking on a real-time thread.
    let wanted = (frames * CHANNELS as usize).min(output.samples.len());
    let samples = &mut output.samples[..wanted];
    let mut starved = false;

    for slot in samples.iter_mut() {
        *slot = match consumer.pop() {
            Ok(sample) => sample,
            Err(_) => {
                // The ring ran dry mid-buffer. Silence is the least bad
                // filler; the counter is what makes it visible.
                starved = true;
                0.0
            }
        };
    }

    if starved {
        // One underrun per buffer, not per sample: a single gap in the audio
        // should read as one event.
        shared.count_underrun();
    }

    let report = output.chain.process(samples, shared.target_gain_db());
    for sample in samples.iter() {
        output.staging.extend(sample.to_le_bytes());
    }

    if report.peak_out > 0.0 {
        shared.report_peak_out(report.peak_out);
    }
    shared.report_gain_reduction_db(report.gain_reduction_db);
}

fn fill_with_silence(staging: &mut VecDeque<u8>, frames: usize) {
    let samples = frames * CHANNELS as usize;
    for _ in 0..samples * BYTES_PER_SAMPLE {
        staging.push_back(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    fn samples_from(staging: &VecDeque<u8>) -> Vec<f32> {
        staging
            .iter()
            .copied()
            .collect::<Vec<u8>>()
            .chunks_exact(BYTES_PER_SAMPLE)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
            .collect()
    }

    /// Pushes `input` through a ring and one `process_to_staging` call, and
    /// returns what was queued for the device.
    fn render(shared: &Shared, output: &mut Output, input: &[f32]) -> Vec<f32> {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(input.len());
        for &sample in input {
            producer.push(sample).unwrap();
        }
        process_to_staging(output, &mut consumer, shared, input.len() / 2);
        let queued = samples_from(&output.staging);
        output.staging.clear();
        queued
    }

    /// At 0 dB, audio below the soft-clip knee (-4.1 dBFS) reaches the
    /// speakers bit for bit, and is metered.
    #[test]
    fn at_0_db_quiet_audio_passes_unchanged_and_is_metered() {
        let shared = Shared::new();
        let mut output = Output::new(48_000, 8, 8);
        let input = [0.5f32, -0.6, 0.25, 0.125];

        assert_eq!(render(&shared, &mut output, &input), input.to_vec());
        assert_eq!(shared.take_peak_out(), 0.6);
        assert_eq!(shared.take_gain_reduction_db(), 0.0);
        assert_eq!(shared.underruns(), 0);
    }

    /// The gain set in `Shared` is what the render path applies, after the
    /// 30 ms ramp.
    #[test]
    fn a_boost_set_in_shared_reaches_the_output() {
        let shared = Shared::new();
        shared.set_target_gain_db(6.0);
        let mut output = Output::new(48_000, 48_000, 8);

        // A second of quiet audio for the smoother to arrive.
        render(&shared, &mut output, &vec![0.1f32; 96_000]);
        let out = render(&shared, &mut output, &[0.1, -0.1]);

        let expected = 0.1 * dsp::meter::from_db(6.0);
        assert!((out[0] - expected).abs() < 1e-6, "{out:?}");
        assert!((out[1] + expected).abs() < 1e-6, "{out:?}");
    }

    /// Full-scale input at the top of the unlocked range still leaves the
    /// render path under the ceiling, and the limiter says it is working.
    #[test]
    fn the_render_path_holds_the_ceiling() {
        let shared = Shared::new();
        shared.set_target_gain_db(dsp::GAIN_MAX_DB_UNLOCKED);
        let mut output = Output::new(48_000, 48_000, 8);
        let ceiling = dsp::meter::from_db(dsp::LIMITER_CEILING_DBFS);

        let loud: Vec<f32> = (0..96_000)
            .map(|i| if i % 4 < 2 { 1.0 } else { -1.0 })
            .collect();
        let out = render(&shared, &mut output, &loud);

        assert!(dsp::meter::peak(&out) <= ceiling);
        // The soft clip ahead of the limiter already caps the signal below
        // full scale, so the limiter only ever closes the last ~1 dB to the
        // -1 dBFS ceiling: its reduction is small by construction.
        let reduction = shared.take_gain_reduction_db();
        assert!(reduction > 0.5 && reduction <= 1.0 + 1e-3, "{reduction}");
    }

    /// A dry ring must not stall the thread: it pads with silence and counts
    /// one underrun for the buffer.
    #[test]
    fn a_dry_ring_pads_with_silence_and_counts_one_underrun() {
        let shared = Shared::new();
        let mut output = Output::new(48_000, 8, 8);
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        producer.push(0.5).unwrap();

        process_to_staging(&mut output, &mut consumer, &shared, 2);

        // Still a full buffer: one real sample, then silence.
        assert_eq!(samples_from(&output.staging), vec![0.5, 0.0, 0.0, 0.0]);
        assert_eq!(
            shared.underruns(),
            1,
            "three missing samples are still one gap"
        );
    }

    #[test]
    fn silence_fills_a_whole_buffer() {
        let mut staging: VecDeque<u8> = VecDeque::new();
        fill_with_silence(&mut staging, 3);

        assert_eq!(staging.len(), 3 * CHANNELS as usize * BYTES_PER_SAMPLE);
        assert!(samples_from(&staging).iter().all(|s| *s == 0.0));
    }

    /// A ring that starts over target must be trimmed down, or capture and
    /// render both running at the device rate hold it there for the whole run.
    #[test]
    fn trimming_brings_a_full_ring_down_to_target() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(1_000);
        while producer.push(0.5).is_ok() {}
        assert_eq!(consumer.slots(), 1_000);

        // 20 frames of stereo is 40 samples.
        trim_to_target(&mut consumer, 20);

        assert_eq!(consumer.slots(), 40);
    }

    /// Below target there is nothing to drop; trimming must not empty the ring.
    #[test]
    fn trimming_leaves_an_underfull_ring_alone() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(1_000);
        for _ in 0..10 {
            producer.push(0.5).unwrap();
        }

        trim_to_target(&mut consumer, 20);

        assert_eq!(consumer.slots(), 10);
    }

    const PACKET: usize = 480;
    const BUFFER: usize = 1_056;
    const TARGET: usize = 960;

    /// How long priming may take in these simulations: the whole-buffer fill
    /// (ticks 0 and 1) and one window. A cold start once spent seconds here.
    const PRIMING_DEADLINE_TICKS: usize = 1 + SETTLE_TICKS as usize;

    /// Simulates priming from a full ring, one engine tick at a time, then
    /// plays on, panicking on an underrun. On each tick capture delivers
    /// `packets(tick)` packets, before render runs if `capture_first`.
    /// Returns the tick playback started on and the fill after each live pull.
    fn prime_then_play(
        capture_first: bool,
        packets: impl Fn(usize) -> usize,
    ) -> (usize, Vec<usize>) {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(48_000);
        while producer.push(0.5).is_ok() {}

        let deliver = |producer: &mut rtrb::Producer<f32>, tick: usize| {
            for _ in 0..packets(tick) * PACKET * CHANNELS as usize {
                let _ = producer.push(0.5);
            }
        };

        // The whole-buffer fill, then the empty request after it.
        let mut requests = vec![BUFFER, 0];
        requests.extend([PACKET; 200]);

        let mut priming = Priming::default();
        let mut started = None;
        let mut after_pull = Vec::new();
        for (tick, frames) in requests.into_iter().enumerate() {
            if capture_first {
                deliver(&mut producer, tick);
            }
            if started.is_some() {
                for _ in 0..frames * CHANNELS as usize {
                    consumer.pop().expect("the ring ran dry: an underrun");
                }
                after_pull.push(queued_frames(&consumer));
            } else if priming.tick(&mut consumer, frames, BUFFER, TARGET) {
                started = Some(tick);
            }
            if !capture_first {
                deliver(&mut producer, tick);
            }
        }
        (started.expect("never left priming"), after_pull)
    }

    /// Playback must start promptly and its low point must sit at the
    /// target: no lower, or the margin is gone; no higher, or it is latency.
    fn assert_primed_well(case: &str, (started, after_pull): (usize, Vec<usize>)) {
        assert!(
            started <= PRIMING_DEADLINE_TICKS,
            "{case}: playback started on tick {started}"
        );
        assert_eq!(
            after_pull.iter().min(),
            Some(&TARGET),
            "{case}: {after_pull:?}"
        );
    }

    /// The order capture and render wake in must not decide the margin for
    /// the rest of the run.
    #[test]
    fn priming_settles_at_target_whichever_thread_runs_first() {
        for capture_first in [true, false] {
            let run = prime_then_play(capture_first, |_| 1);
            assert_primed_well(&format!("capture_first={capture_first}"), run);
        }
    }

    /// A packet capture misses while priming is made up before playback,
    /// rather than costing a period of margin for the rest of the run.
    #[test]
    fn a_packet_missed_while_priming_is_not_locked_in() {
        for capture_first in [true, false] {
            // Ticks 0 and 1 are the whole-buffer fill and the window follows.
            // When render runs first, the last window tick's packet lands
            // after playback has started, so it is not a priming miss.
            for missed in 2..1 + SETTLE_TICKS as usize {
                let run = prime_then_play(capture_first, |tick| usize::from(tick != missed));
                assert_primed_well(
                    &format!("capture_first={capture_first} missed={missed}"),
                    run,
                );
            }
        }
    }

    /// A cable fresh from idle delivers two packets on one tick and none on
    /// the next. This is the pattern that kept a cold start silent for
    /// seconds; it must prime as fast as steady delivery does.
    #[test]
    fn bursty_capture_from_a_cold_start_primes_promptly() {
        for capture_first in [true, false] {
            for phase in [0, 1] {
                let run =
                    prime_then_play(capture_first, |tick| if tick % 2 == phase { 2 } else { 0 });
                assert_primed_well(&format!("capture_first={capture_first} phase={phase}"), run);
            }
        }
    }

    /// The whole-buffer fill is not a period and must never prime.
    #[test]
    fn the_initial_buffer_fill_does_not_count_as_a_period() {
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(48_000);
        while producer.push(0.5).is_ok() {}

        let mut priming = Priming::default();
        assert!(!priming.tick(&mut consumer, BUFFER, BUFFER, TARGET));
        assert!(!priming.tick(&mut consumer, 0, BUFFER, TARGET));
        assert_eq!(consumer.slots(), 48_000, "nothing pulled outside a period");
    }
}
