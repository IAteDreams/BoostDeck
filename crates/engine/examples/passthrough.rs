//! Pass-through spike: CABLE Output -> ring -> real speakers, at unity gain.
//!
//! Coding-order step 3. No GUI, no DSP, no routing: this exists to prove the
//! audio path is stable for hours before anything is built on it.
//!
//! ```text
//! cargo run -p engine --example passthrough              # until Ctrl+C
//! cargo run -p engine --example passthrough -- 600       # ten minutes
//! cargo run -p engine --example passthrough -- 600 <id>  # pick the output
//! ```
//!
//! Set CABLE Input as the Windows default output by hand first (Sound
//! settings), so apps play into the cable. Routing that automatically is
//! step 6.
//!
//! Use headphones at low volume for the first runs. Nothing here scales the
//! samples, but a bug in the buffer path can still produce full-scale noise.

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use engine::rt::{CHANNELS, RtError, Shared};
use engine::{capture, devices, render};
use rtrb::RingBuffer;
use wasapi::{DeviceEnumerator, initialize_mta};

/// How often the stats line is printed.
const REPORT_EVERY: Duration = Duration::from_secs(5);

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let run_for: Option<u64> = match args.next() {
        Some(arg) => Some(arg.parse().context("the first argument is seconds to run")?),
        None => None,
    };
    let output_id = args.next();

    initialize_mta().ok().context("initialising COM (MTA)")?;
    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;

    // The cable has to be there, and usable, before anything else.
    let cable = devices::find_cable(&enumerator)?;
    if let Some(problem) = cable.problem() {
        bail!("VB-Cable is not ready: {problem}");
    }
    let cable_output = cable.output.context("CABLE Output went missing")?;

    let output = pick_output(&enumerator, output_id.as_deref())?;

    // Both streams run at the real device's rate, so the ring never has to
    // resample. Windows converts for the cable if it is set to another rate.
    let sample_rate = capture::device_sample_rate(&enumerator, &output.id)?;

    let target_fill_frames = sample_rate * dsp::drift::RING_TARGET_MS / 1_000;
    let ring_capacity =
        (sample_rate as f32 * dsp::drift::RING_CAPACITY_SECS) as usize * CHANNELS as usize;

    println!("Capture : {} [{}]", cable_output.name, cable_output.id);
    println!("Render  : {} [{}]", output.name, output.id);
    println!("Rate    : {sample_rate} Hz, {CHANNELS} ch");
    println!(
        "Ring    : {ring_capacity} samples capacity, target fill {target_fill_frames} frames ({} ms)",
        dsp::drift::RING_TARGET_MS
    );
    match run_for {
        Some(secs) => println!("Running for {secs} s. Ctrl+C stops it sooner.\n"),
        None => println!("Running until Ctrl+C.\n"),
    }

    let (producer, consumer) = RingBuffer::<f32>::new(ring_capacity);
    let shared = Arc::new(Shared::new());

    // Render starts first so the device is ready to be fed the moment capture
    // produces anything; it outputs silence until the ring hits its target.
    let render_handle = {
        let shared = Arc::clone(&shared);
        let config = render::RenderConfig {
            device_id: output.id.clone(),
            sample_rate,
            target_fill_frames,
        };
        thread::Builder::new()
            .name("boostdeck-render".to_owned())
            .spawn(move || render::run(config, consumer, shared))
            .context("spawning the render thread")?
    };

    let capture_handle = {
        let shared = Arc::clone(&shared);
        let config = capture::CaptureConfig {
            device_id: cable_output.id.clone(),
            sample_rate,
        };
        thread::Builder::new()
            .name("boostdeck-capture".to_owned())
            .spawn(move || capture::run(config, producer, shared))
            .context("spawning the capture thread")?
    };

    report_until_done(&shared, run_for);

    shared.request_stop();
    let capture_result = capture_handle.join();
    let render_result = render_handle.join();

    println!();
    report_thread("capture", capture_result);
    report_thread("render", render_result);

    let underruns = shared.underruns();
    println!(
        "\nTotals: {underruns} underruns, {} overruns, {} discontinuities",
        shared.overruns(),
        shared.discontinuities()
    );
    if underruns == 0 {
        println!("No underruns: the audio path held.");
    } else {
        println!("Underruns mean audible gaps. Raise the ring target and try again.");
    }

    Ok(())
}

/// Prints a stats line every few seconds until the time is up, the threads
/// report an error, or the process is interrupted.
fn report_until_done(shared: &Shared, run_for: Option<u64>) {
    let started = Instant::now();
    let deadline = run_for.map(|secs| started + Duration::from_secs(secs));

    loop {
        thread::sleep(REPORT_EVERY);

        let elapsed = started.elapsed().as_secs();
        println!(
            "{elapsed:>5}s  fill {:>5} frames  in {:>6}  out {:>6}  under {:>3}  over {:>3}  disc {:>3}",
            shared.ring_fill_frames(),
            dbfs(shared.take_peak_in()),
            dbfs(shared.take_peak_out()),
            shared.underruns(),
            shared.overruns(),
            shared.discontinuities(),
        );

        let error = shared.error();
        if error != RtError::None {
            println!("\nAn audio thread reported {error:?}; stopping.");
            return;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return;
        }
    }
}

/// Peak as dBFS, which is the scale the limiter and the meters work in.
fn dbfs(peak: f32) -> String {
    if peak <= 0.0 {
        return "  -inf".to_owned();
    }
    format!("{:>6.1}", 20.0 * peak.log10())
}

fn report_thread(name: &str, result: thread::Result<Result<()>>) {
    match result {
        Ok(Ok(())) => println!("The {name} thread finished cleanly."),
        Ok(Err(error)) => println!("The {name} thread failed: {error:#}"),
        Err(_) => println!("The {name} thread panicked."),
    }
}

/// Resolves the output device: the one named on the command line, else the
/// first real candidate. Never a virtual cable.
fn pick_output(
    enumerator: &DeviceEnumerator,
    requested: Option<&str>,
) -> Result<devices::DeviceInfo> {
    let candidates = devices::output_candidates(enumerator)?;

    if let Some(wanted) = requested {
        // Accept an id or any part of a friendly name, so the ids printed by
        // the devices example can be pasted in as-is.
        return candidates
            .into_iter()
            .find(|d| d.id == wanted || d.name.contains(wanted))
            .with_context(|| format!("no real output device matches {wanted:?}"));
    }

    let mut candidates = candidates.into_iter();
    let first = candidates
        .next()
        .context("no active output device that is not a virtual cable")?;

    if candidates.len() > 0 {
        println!("Several outputs available; using {}.", first.name);
        println!("Pass an id or a name fragment as the second argument to choose.\n");
    }
    Ok(first)
}
