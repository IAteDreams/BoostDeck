//! Capture thread: reads CABLE Output into the ring buffer.
//!
//! Coding-order step 3. This is a real-time thread, so once the stream is
//! running it must not allocate, lock, log, or touch a file or a channel.
//! Every buffer is sized before `start_stream`, and the only way out is the
//! `Shared` atomics.

use std::collections::VecDeque;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use rtrb::Producer;
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

use crate::rt::{BYTES_PER_SAMPLE, CHANNELS, ProAudioPriority, RtError, Shared};

/// How long to wait for the device to signal a buffer before treating the
/// stream as dead. Generous next to a device period of a few milliseconds,
/// short enough that a vanished device is noticed quickly.
const EVENT_TIMEOUT_MS: u32 = 2_000;

/// Spare room in the byte staging queue, as a multiple of one device buffer.
/// The queue is drained to empty every cycle; this only has to absorb a cycle
/// that returns more than one buffer's worth.
const STAGING_BUFFERS: usize = 8;

pub struct CaptureConfig {
    /// Endpoint id of CABLE Output. An id, not a friendly name: names are
    /// neither unique nor stable.
    pub device_id: String,
    /// Opened at the render device's rate so both ends of the ring agree.
    pub sample_rate: u32,
}

/// Runs until `Shared::request_stop` is set or the stream fails.
///
/// Call this as the entire body of a dedicated thread: it initialises COM for
/// the thread and registers it with MMCSS.
pub fn run(config: CaptureConfig, producer: Producer<f32>, shared: Arc<Shared>) -> Result<()> {
    let result = run_inner(config, producer, &shared);
    if let Err(error) = &result {
        // The control thread polls for this; it is the only channel a
        // real-time thread has.
        shared.set_error(match error.downcast_ref::<Timeout>() {
            Some(_) => RtError::Timeout,
            None => RtError::Capture,
        });
    }
    result
}

/// Marker so a stalled device is distinguishable from an outright failure.
#[derive(Debug)]
struct Timeout;

impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the capture device stopped signalling its event")
    }
}

impl std::error::Error for Timeout {}

fn run_inner(config: CaptureConfig, mut producer: Producer<f32>, shared: &Shared) -> Result<()> {
    // COM is per-thread, so this thread initialises its own apartment and
    // resolves its own device rather than sharing one across threads.
    initialize_mta().ok().context("initialising COM (MTA)")?;
    let _priority = ProAudioPriority::register()?;

    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;
    let device = enumerator
        .get_device(&config.device_id)
        .with_context(|| format!("opening capture device {}", config.device_id))?;
    let mut audio_client = device
        .get_iaudioclient()
        .context("getting the capture audio client")?;

    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        config.sample_rate as usize,
        CHANNELS as usize,
        None,
    );
    let block_align = format.get_blockalign() as usize;

    // `autoconvert` lets Windows resample if the cable runs at another rate,
    // so a mismatched VB-Cable setting costs a conversion rather than failing.
    let (_default_period, min_period) = audio_client
        .get_device_period()
        .context("reading the capture device period")?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: min_period,
    };
    audio_client
        .initialize_client(&format, &Direction::Capture, &mode)
        .context("initialising the capture client")?;

    let event = audio_client
        .set_get_eventhandle()
        .context("getting the capture event handle")?;
    let buffer_frames = audio_client
        .get_buffer_size()
        .context("reading the capture buffer size")? as usize;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .context("getting the capture client")?;

    // Allocated once, before the stream starts, and never grown afterwards.
    let mut staging: VecDeque<u8> =
        VecDeque::with_capacity(block_align * buffer_frames * STAGING_BUFFERS);

    audio_client
        .start_stream()
        .context("starting the capture stream")?;

    let outcome = capture_loop(
        &capture_client,
        &event,
        &mut staging,
        &mut producer,
        shared,
    );

    // Stop even when the loop failed, so the endpoint is released either way.
    let _ = audio_client.stop_stream();
    outcome
}

fn capture_loop(
    capture_client: &wasapi::AudioCaptureClient,
    event: &wasapi::Handle,
    staging: &mut VecDeque<u8>,
    producer: &mut Producer<f32>,
    shared: &Shared,
) -> Result<()> {
    while !shared.should_stop() {
        if event.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            return Err(Timeout.into());
        }

        let info = capture_client
            .read_from_device_to_deque(staging)
            .context("reading from the capture device")?;

        // Windows telling us it already lost audio. Worth counting separately
        // from our own ring overruns: this one happened upstream of us.
        if info.flags.data_discontinuity {
            shared.count_discontinuity();
        }

        drain_into_ring(staging, producer, shared);
    }
    Ok(())
}

/// Moves whole samples from the byte queue into the ring, tracking the input
/// peak on the way past.
///
/// The byte-at-a-time pop keeps this readable and allocates nothing. A deque
/// holds its contents as one or two contiguous runs, so if this ever shows up
/// in a profile the faster version is `as_slices` plus `rtrb`'s chunk API.
fn drain_into_ring(staging: &mut VecDeque<u8>, producer: &mut Producer<f32>, shared: &Shared) {
    let mut peak = 0.0f32;

    while staging.len() >= BYTES_PER_SAMPLE {
        let mut bytes = [0u8; BYTES_PER_SAMPLE];
        for byte in bytes.iter_mut() {
            // The length check above guarantees these are present. `unwrap_or`
            // rather than `unwrap` because a real-time thread must not panic.
            *byte = staging.pop_front().unwrap_or(0);
        }

        let sample = f32::from_le_bytes(bytes);
        peak = peak.max(sample.abs());

        if producer.push(sample).is_err() {
            // The ring is full: render is not keeping up, or has stopped. Drop
            // the sample rather than block, and let the counter say so.
            shared.count_overrun();
        }
    }

    if peak > 0.0 {
        shared.report_peak_in(peak);
    }
}

/// Reads an endpoint's current mix rate without opening a stream on it.
///
/// Used to decide the rate both streams run at, which the guide fixes to the
/// real output device's.
pub fn device_sample_rate(enumerator: &DeviceEnumerator, device_id: &str) -> Result<u32> {
    let device = enumerator
        .get_device(device_id)
        .with_context(|| format!("opening device {device_id}"))?;
    let format = device
        .get_device_format()
        .context("reading the device mix format")?;
    Ok(format.get_samplespersec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    /// The conversion the drain performs, in isolation: little-endian f32
    /// bytes in, samples out, loudest magnitude reported.
    #[test]
    fn drains_bytes_into_samples_and_meters_the_peak() {
        let shared = Shared::new();
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);

        let mut staging: VecDeque<u8> = VecDeque::new();
        for sample in [0.5f32, -0.75, 0.25] {
            staging.extend(sample.to_le_bytes());
        }

        drain_into_ring(&mut staging, &mut producer, &shared);

        assert_eq!(consumer.pop(), Ok(0.5));
        assert_eq!(consumer.pop(), Ok(-0.75));
        assert_eq!(consumer.pop(), Ok(0.25));
        // Magnitude, so the -0.75 wins.
        assert_eq!(shared.take_peak_in(), 0.75);
        assert_eq!(shared.overruns(), 0);
    }

    /// A trailing partial sample must stay in the queue for the next buffer,
    /// not be padded into a bogus value.
    #[test]
    fn a_partial_sample_is_left_for_next_time() {
        let shared = Shared::new();
        let (mut producer, mut consumer) = RingBuffer::<f32>::new(8);

        let mut staging: VecDeque<u8> = VecDeque::new();
        staging.extend(1.0f32.to_le_bytes());
        staging.extend([0u8, 1, 2]); // three bytes: not a whole sample

        drain_into_ring(&mut staging, &mut producer, &shared);

        assert_eq!(consumer.pop(), Ok(1.0));
        assert_eq!(staging.len(), 3, "the partial sample was consumed");
    }

    /// When render stops consuming, capture drops samples and counts them
    /// rather than blocking the real-time thread.
    #[test]
    fn a_full_ring_counts_overruns_instead_of_blocking() {
        let shared = Shared::new();
        let (mut producer, _consumer) = RingBuffer::<f32>::new(2);

        let mut staging: VecDeque<u8> = VecDeque::new();
        for sample in [0.1f32, 0.2, 0.3, 0.4] {
            staging.extend(sample.to_le_bytes());
        }

        drain_into_ring(&mut staging, &mut producer, &shared);

        assert!(staging.is_empty(), "the queue should still be drained");
        assert_eq!(shared.overruns(), 2, "two samples past a ring of two");
    }

    #[test]
    fn an_empty_queue_reports_no_peak() {
        let shared = Shared::new();
        let (mut producer, _consumer) = RingBuffer::<f32>::new(4);
        let mut staging: VecDeque<u8> = VecDeque::new();

        drain_into_ring(&mut staging, &mut producer, &shared);

        assert_eq!(shared.take_peak_in(), 0.0);
    }
}
