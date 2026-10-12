//! Render thread: ring buffer to the real output device.
//!
//! Coding-order step 3, at unity gain. This is a real-time thread, so once the
//! stream is running it must not allocate, lock, log, or touch a file or a
//! channel.
//!
//! Step 4 inserts the DSP where `copy_to_staging` currently copies straight
//! through: read the target gain from `Shared`, run gain -> soft clip ->
//! limiter, then report both peak meters. Until then nothing scales the
//! samples, which is what keeps the spike safe to listen to.

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

    // Allocated once, before the stream starts, and never grown afterwards.
    let mut staging: VecDeque<u8> =
        VecDeque::with_capacity(block_align * buffer_frames * STAGING_BUFFERS);

    audio_client
        .start_stream()
        .context("starting the render stream")?;

    let outcome = render_loop(
        &audio_client,
        &render_client,
        &event,
        &mut staging,
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
    staging: &mut VecDeque<u8>,
    consumer: &mut Consumer<f32>,
    shared: &Shared,
    target_fill_frames: u32,
) -> Result<()> {
    // Silence goes out until the ring has filled to its target once. Starting
    // to pull from a nearly empty ring would underrun immediately.
    let mut primed = false;

    while !shared.should_stop() {
        let frames = audio_client
            .get_available_space_in_frames()
            .context("reading the available render space")? as usize;

        let queued_frames = consumer.slots() / CHANNELS as usize;
        shared.set_ring_fill_frames(queued_frames as u32);

        if !primed && queued_frames >= target_fill_frames as usize {
            primed = true;
            // Never trim below the buffer the device is asking for right now:
            // that first request can be larger than the target, and cutting to
            // the target exactly would underrun on the very first pull.
            trim_to_target(consumer, target_fill_frames.max(frames as u32));
            // Everything counted so far belongs to filling the ring, not to
            // playback. Reset so the totals describe the steady state.
            shared.reset_counters();
        }

        if primed {
            copy_to_staging(staging, consumer, shared, frames);
        } else {
            fill_with_silence(staging, frames);
        }

        render_client
            .write_to_device_from_deque(frames, staging, None)
            .context("writing to the render device")?;

        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            return Err(Timeout.into());
        }
    }
    Ok(())
}

/// Drops everything in the ring past `keep_frames`.
///
/// At startup the ring can be far past target: the cable hands over a backlog
/// while the render stream is still being opened, and capture and render then
/// both run at the device rate, so a ring that starts full *stays* full. That
/// is a stable equilibrium at the worst possible latency - a nearly full
/// half-second ring measured 488 ms of delay before this was added.
///
/// Discarding is audible in principle, but this runs once, before the first
/// sample is played. Ongoing drift is corrected a frame at a time in step 5,
/// which is inaudible; this is deliberately not that.
fn trim_to_target(consumer: &mut Consumer<f32>, keep_frames: u32) {
    let target_samples = keep_frames as usize * CHANNELS as usize;
    while consumer.slots() > target_samples {
        if consumer.pop().is_err() {
            break;
        }
    }
}

/// Fills one device buffer from the ring, padding with silence if the ring
/// runs dry, and meters the output.
///
/// Step 4 replaces the straight copy here with gain -> soft clip -> limiter.
fn copy_to_staging(
    staging: &mut VecDeque<u8>,
    consumer: &mut Consumer<f32>,
    shared: &Shared,
    frames: usize,
) {
    let wanted = frames * CHANNELS as usize;
    let mut peak = 0.0f32;
    let mut starved = false;

    for _ in 0..wanted {
        let sample = match consumer.pop() {
            Ok(sample) => sample,
            Err(_) => {
                // The ring ran dry mid-buffer. Silence is the least bad
                // filler; the counter is what makes it visible.
                starved = true;
                0.0
            }
        };

        peak = peak.max(sample.abs());
        staging.extend(sample.to_le_bytes());
    }

    if starved {
        // One underrun per buffer, not per sample: a single gap in the audio
        // should read as one event.
        shared.count_underrun();
    }
    if peak > 0.0 {
        shared.report_peak_out(peak);
    }
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

    #[test]
    fn copies_the_ring_into_bytes_and_meters_the_peak() {
        let shared = Shared::new();
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        for sample in [0.5f32, -0.75, 0.25, 0.125] {
            producer.push(sample).unwrap();
        }

        let mut staging: VecDeque<u8> = VecDeque::new();
        // Two frames of stereo is four samples.
        copy_to_staging(&mut staging, &mut consumer, &shared, 2);

        assert_eq!(samples_from(&staging), vec![0.5, -0.75, 0.25, 0.125]);
        assert_eq!(shared.take_peak_out(), 0.75);
        assert_eq!(shared.underruns(), 0);
    }

    /// A dry ring must not stall the thread: it pads with silence and counts
    /// one underrun for the buffer.
    #[test]
    fn a_dry_ring_pads_with_silence_and_counts_one_underrun() {
        let shared = Shared::new();
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        producer.push(1.0).unwrap();

        let mut staging: VecDeque<u8> = VecDeque::new();
        copy_to_staging(&mut staging, &mut consumer, &shared, 2);

        // Still a full buffer: one real sample, then silence.
        assert_eq!(samples_from(&staging), vec![1.0, 0.0, 0.0, 0.0]);
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

    /// Unity gain is the whole point of the step-3 spike: whatever came out of
    /// the cable must reach the speakers unscaled.
    #[test]
    fn the_spike_does_not_change_the_samples() {
        let shared = Shared::new();
        let input = [0.9f32, -0.9, 0.3, -0.3];
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);
        for sample in input {
            producer.push(sample).unwrap();
        }

        let mut staging: VecDeque<u8> = VecDeque::new();
        copy_to_staging(&mut staging, &mut consumer, &shared, 2);

        assert_eq!(samples_from(&staging), input.to_vec());
    }
}
