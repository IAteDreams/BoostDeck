//! Pass-through spike: CABLE Output -> ring -> DSP -> real speakers.
//!
//! Coding-order steps 3 and 4. No GUI, no routing: this exists to prove the
//! audio path is stable for hours, and the boost works, before anything is
//! built on it.
//!
//! ```text
//! cargo run -p engine --example passthrough                    # until Ctrl+C
//! cargo run -p engine --example passthrough -- 600             # ten minutes
//! cargo run -p engine --example passthrough -- 600 <id>        # pick the output
//! cargo run -p engine --example passthrough -- 600 --gain-db 6 # boost by 6 dB
//! cargo run -p engine --example passthrough -- 600 --route     # switch the default too
//! ```
//!
//! Without `--route`, set CABLE Input as the Windows default output by hand
//! first (Sound settings), so apps play into the cable. With it, the example
//! switches the default to CABLE Input itself and puts the original back on
//! exit, Ctrl+C included. It records the switch in the app's `settings.json`
//! first, exactly as the app will, so a run killed from Task Manager is
//! undone by the next launch of BoostDeck (step 6's "done when").
//!
//! Gain defaults to 0 dB and is capped at `dsp::GAIN_MAX_DB` (+12 dB), the
//! same cap the slider has; the limiter is always on. Use headphones at low
//! volume for the first runs: a bug in the buffer path can still produce
//! full-scale noise.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use engine::rt::{CHANNELS, RtError, Shared};
use engine::{capture, devices, render, routing};
use rtrb::RingBuffer;
use wasapi::{DeviceEnumerator, initialize_mta};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows_core::BOOL;

/// How often the stats line is printed.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// Set by Ctrl+C, so the run ends through the normal path and restores the
/// default output rather than dying with the cable still in place.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let gain_db = take_gain_db(&mut args)?;
    let route = take_flag(&mut args, "--route");
    let mut args = args.into_iter();
    let run_for: Option<u64> = match args.next() {
        Some(arg) => Some(
            arg.parse()
                .context("the first argument is seconds to run")?,
        ),
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
        "Gain    : {gain_db:+.1} dB, limiter ceiling {:.1} dBFS",
        dsp::LIMITER_CEILING_DBFS
    );
    println!(
        "Ring    : {ring_capacity} samples capacity, target fill {target_fill_frames} frames ({} ms)",
        dsp::drift::RING_TARGET_MS
    );
    // SAFETY: registers a plain function that only stores to an atomic.
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl_c), true) }
        .context("installing the Ctrl+C handler")?;
    routing::install_panic_hook();

    let journal = route.then(Journal::new).transpose()?;
    if let Some(journal) = &journal {
        let routed =
            routing::route_to_cable(&enumerator, journal.saved_original()?.as_deref(), |id| {
                journal.record(Some(id))
            })?;
        println!(
            "Routing : default output {} -> {} (recorded in {})",
            routed.original.name,
            routed.cable_input.name,
            journal.path.display()
        );
    }

    match run_for {
        Some(secs) => println!("Running for {secs} s. Ctrl+C stops it sooner.\n"),
        None => println!("Running until Ctrl+C.\n"),
    }

    let played = play(
        &cable_output,
        &output,
        sample_rate,
        target_fill_frames,
        ring_capacity,
        gain_db,
        run_for,
    );

    // Restore whatever happened while playing: leaving the cable as the
    // default is the one failure the user cannot hear their way out of.
    if let Some(journal) = &journal {
        let restored = routing::restore(&enumerator, journal.saved_original()?.as_deref())?;
        journal.record(None)?;
        println!(
            "Routing : default output restored to {}{}",
            restored.device.name,
            if restored.fell_back {
                " (the original was gone)"
            } else {
                ""
            }
        );
    }
    played
}

/// Runs the streams until the time is up, then prints the totals.
fn play(
    cable_output: &devices::DeviceInfo,
    output: &devices::DeviceInfo,
    sample_rate: u32,
    target_fill_frames: u32,
    ring_capacity: usize,
    gain_db: f32,
    run_for: Option<u64>,
) -> Result<()> {
    let (producer, consumer) = RingBuffer::<f32>::new(ring_capacity);
    let shared = Arc::new(Shared::new());
    // Set before the streams start; the render thread still ramps up to it
    // from unity over the smoothing time rather than starting loud.
    shared.set_target_gain_db(gain_db);

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

    let fill_range = report_until_done(&shared, run_for);

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

    // The step-5 bar: fill held within the drift tolerance of target.
    let tolerance_frames = sample_rate * dsp::drift::DRIFT_TOLERANCE_MS / 1_000;
    let band = (
        target_fill_frames.saturating_sub(tolerance_frames),
        target_fill_frames + tolerance_frames,
    );
    match fill_range {
        Some((low, high)) => {
            let held = low >= band.0 && high <= band.1;
            println!(
                "Fill    : {low}..{high} frames over the run, band {}..{} (target {} frames +/- {} ms): {}",
                band.0,
                band.1,
                target_fill_frames,
                dsp::drift::DRIFT_TOLERANCE_MS,
                if held { "held" } else { "NOT held" }
            );
        }
        None => println!("Fill    : no playback recorded"),
    }
    println!(
        "Drift   : {} frames dropped, {} repeated",
        shared.drift_drops(),
        shared.drift_repeats()
    );
    if underruns == 0 {
        println!("No underruns: the audio path held.");
    } else {
        println!("Underruns mean audible gaps. Raise the ring target and try again.");
    }

    Ok(())
}

/// Prints a stats line every few seconds until the time is up, the threads
/// report an error, or the process is interrupted. Returns the lowest and
/// highest ring fill seen across the whole run, if any was recorded.
///
/// The bracketed range on each line is every fill since the previous line,
/// not a snapshot: a snapshot every 5 s once hid a fill sitting a packet low.
fn report_until_done(shared: &Shared, run_for: Option<u64>) -> Option<(u32, u32)> {
    let started = Instant::now();
    let deadline = run_for.map(|secs| started + Duration::from_secs(secs));
    let mut worst: Option<(u32, u32)> = None;

    let mut next_report = started;
    loop {
        // Sleep in small steps so Ctrl+C is answered at once.
        next_report += REPORT_EVERY;
        while Instant::now() < next_report {
            if INTERRUPTED.load(Ordering::Relaxed) {
                println!("\nInterrupted.");
                return worst;
            }
            thread::sleep(Duration::from_millis(50));
        }

        let elapsed = started.elapsed().as_secs();
        let range = shared.take_ring_fill_range();
        if let Some((low, high)) = range {
            worst = Some(match worst {
                Some((worst_low, worst_high)) => (worst_low.min(low), worst_high.max(high)),
                None => (low, high),
            });
        }
        let (low, high) = range.map_or(("-".to_owned(), "-".to_owned()), |(low, high)| {
            (low.to_string(), high.to_string())
        });
        println!(
            "{elapsed:>5}s  fill avg {:>6.0} [{low:>5}..{high:>5}]  in {:>6}  out {:>6}  gr {:>4.1}  drop {:>3} rep {:>3}  under {:>3}  over {:>3}  disc {:>3}",
            shared.ring_fill_average(),
            dbfs(shared.take_peak_in()),
            dbfs(shared.take_peak_out()),
            shared.take_gain_reduction_db(),
            shared.drift_drops(),
            shared.drift_repeats(),
            shared.underruns(),
            shared.overruns(),
            shared.discontinuities(),
        );

        let error = shared.error();
        if error != RtError::None {
            println!("\nAn audio thread reported {error:?}; stopping.");
            return worst;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return worst;
        }
    }
}

/// Removes `--gain-db <dB>` from the arguments, wherever it is, and returns
/// the gain: 0 dB when absent. Refuses anything outside the slider's range
/// rather than quietly clamping it.
fn take_gain_db(args: &mut Vec<String>) -> Result<f32> {
    let Some(flag) = args.iter().position(|arg| arg == "--gain-db") else {
        return Ok(0.0);
    };
    args.remove(flag);
    if flag >= args.len() {
        bail!("--gain-db needs a value in dB");
    }
    let value = args.remove(flag);
    let gain_db: f32 = value
        .parse()
        .with_context(|| format!("--gain-db {value:?} is not a number"))?;
    if !(dsp::GAIN_MIN_DB..=dsp::GAIN_MAX_DB).contains(&gain_db) {
        bail!(
            "--gain-db {gain_db} is outside {} to +{} dB",
            dsp::GAIN_MIN_DB,
            dsp::GAIN_MAX_DB
        );
    }
    Ok(gain_db)
}

/// Removes `flag` from the arguments, wherever it is, and says whether it was
/// there.
fn take_flag(args: &mut Vec<String>, flag: &str) -> bool {
    let before = args.len();
    args.retain(|arg| arg != flag);
    args.len() != before
}

unsafe extern "system" fn on_ctrl_c(_ctrl_type: u32) -> BOOL {
    INTERRUPTED.store(true, Ordering::Relaxed);
    // Handled: Windows must not kill the process before it restores.
    true.into()
}

/// The routing record in the app's `settings.json`: the two fields the app's
/// launch check reads. The app owns that file's schema, so every other field
/// is carried through untouched.
struct Journal {
    path: PathBuf,
}

impl Journal {
    fn new() -> Result<Self> {
        let (qualifier, organisation, application) = presets::PROJECT_DIRS;
        let dirs = directories::ProjectDirs::from(qualifier, organisation, application)
            .context("Windows did not report an app-data folder")?;
        Ok(Self {
            path: dirs.config_dir().join("settings.json"),
        })
    }

    fn read(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(serde_json::Value::Object(map)) => Ok(map),
                _ => bail!("{} is not a settings object", self.path.display()),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", self.path.display())),
        }
    }

    /// The original a previous run recorded and never cleared, if any.
    fn saved_original(&self) -> Result<Option<String>> {
        let settings = self.read()?;
        let active = settings.get("routing_active") == Some(&serde_json::Value::Bool(true));
        Ok(active
            .then(|| {
                settings
                    .get("original_default_device_id")?
                    .as_str()
                    .map(str::to_owned)
            })
            .flatten())
    }

    /// `Some(id)` records a route to undo; `None` clears it. Atomic, like the
    /// app's own save.
    fn record(&self, original: Option<&str>) -> Result<()> {
        let mut settings = self.read()?;
        settings.insert("routing_active".to_owned(), original.is_some().into());
        settings.insert("original_default_device_id".to_owned(), original.into());
        write_atomically(&self.path, &serde_json::to_string_pretty(&settings)?)
    }
}

fn write_atomically(path: &Path, text: &str) -> Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let temp = path.with_extension("json.tmp");
    let mut file =
        std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("writing {}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
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
