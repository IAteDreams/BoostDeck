//! The engine as the app will drive it: the control thread routes, plays,
//! and rides out device changes on its own, with no GUI.
//!
//! Coding-order step 7. Run it, then unplug headphones, change the speakers'
//! sample rate in Sound settings, or sleep and resume the PC; each one should
//! show up as events and playback should come back by itself.
//!
//! ```text
//! cargo run -p engine --example control                         # until Ctrl+C
//! cargo run -p engine --example control -- 600                  # ten minutes
//! cargo run -p engine --example control -- 600 --output Realtek # pick the output
//! cargo run -p engine --example control -- 600 --gain-db 6      # boost by 6 dB
//! cargo run -p engine --example control -- 60 --fail-at 10      # fake a dead stream
//! ```
//!
//! It switches the Windows default to CABLE Input itself and records that in
//! the app's `settings.json`, exactly as the app will. Where the app would
//! ask the user - "your headphones are back, switch to them?" - this example
//! says yes on their behalf.
//!
//! `--fail-at <seconds>` raises the render thread's error flag once, the way
//! a format change or a sleep does, to exercise the rebuild without touching
//! any settings.

mod common;

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use common::{Journal, dbfs, take_gain_db};
use crossbeam_channel::RecvTimeoutError;
use engine::control::{Engine, EngineCommand, EngineEvent, StartOptions};
use engine::devices;
use engine::routing::{self, RoutingJournal as _};
use engine::rt::RtError;
use wasapi::{DeviceEnumerator, initialize_mta};

/// How often the stats line is printed.
const REPORT_EVERY: Duration = Duration::from_secs(5);

/// How long to wait for the engine to confirm the default is restored.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let gain_db = take_gain_db(&mut args)?;
    let output = take_value(&mut args, "--output")?;
    let mut fail_at = take_value(&mut args, "--fail-at")?
        .map(|secs| secs.parse().map(Duration::from_secs))
        .transpose()
        .context("--fail-at takes seconds")?;
    let run_for = match args.first() {
        Some(arg) => Some(Duration::from_secs(
            arg.parse()
                .context("the first argument is seconds to run")?,
        )),
        None => None,
    };

    common::catch_ctrl_c()?;
    routing::install_panic_hook();

    let output_id = match output {
        Some(wanted) => Some(find_output(&wanted)?),
        None => None,
    };

    let journal = Journal::new()?;
    println!("Routing record: {}", journal.path.display());
    if let Some(saved) = journal.saved_original() {
        println!("A previous run left a route behind; its original was {saved}.");
    }

    let engine = Engine::spawn(Box::new(journal))?;
    engine.shared().set_target_gain_db(gain_db);
    println!("Gain   : {gain_db:+.1} dB");
    match run_for {
        Some(time) => println!(
            "Running for {} s. Ctrl+C stops it sooner.\n",
            time.as_secs()
        ),
        None => println!("Running until Ctrl+C.\n"),
    }
    engine.send(EngineCommand::Start(StartOptions {
        output_id,
        ..StartOptions::default()
    }));

    let started = Instant::now();
    let mut next_report = started + REPORT_EVERY;
    loop {
        if common::interrupted() {
            println!("\nInterrupted.");
            break;
        }
        if run_for.is_some_and(|time| started.elapsed() >= time) {
            break;
        }
        match engine.events().recv_timeout(Duration::from_millis(50)) {
            Ok(event) => {
                print_event(started, &event);
                // Standing in for the app's prompts, always answering yes.
                match event {
                    // "Your headphones are back. Switch to them?"
                    EngineEvent::OutputReturned(device) => {
                        println!("{:>7}  switching back to {}", stamp(started), device.name);
                        engine.send(EngineCommand::SetOutput(device.id));
                    }
                    // "You picked another output. Boost that one instead?"
                    EngineEvent::DefaultMovedOffCable(Some(device))
                        if !device.is_virtual_cable() =>
                    {
                        println!(
                            "{:>7}  re-routing, playing to {}",
                            stamp(started),
                            device.name
                        );
                        engine.send(EngineCommand::Start(StartOptions {
                            output_id: Some(device.id),
                            ..StartOptions::default()
                        }));
                    }
                    _ => {}
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                println!("The control thread has gone.");
                break;
            }
        }
        if fail_at.is_some_and(|at| started.elapsed() >= at) {
            fail_at = None;
            println!("{:>7}  faking a render failure", stamp(started));
            engine.shared().set_error(RtError::Render);
        }
        if Instant::now() >= next_report {
            next_report += REPORT_EVERY;
            report(started, &engine);
        }
    }

    println!("\nStopping.");
    engine.send(EngineCommand::Stop);
    let deadline = Instant::now() + STOP_TIMEOUT;
    while let Ok(event) = engine
        .events()
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
    {
        print_event(started, &event);
        if matches!(event, EngineEvent::Restored(_) | EngineEvent::Error(_)) {
            break;
        }
    }
    let shared = engine.shared().clone();
    drop(engine);
    println!(
        "\nTotals since the last (re)start: {} underruns, {} overruns, {} discontinuities",
        shared.underruns(),
        shared.overruns(),
        shared.discontinuities()
    );
    // Let the log reach the terminal before the process ends.
    thread::sleep(Duration::from_millis(50));
    Ok(())
}

fn stamp(started: Instant) -> String {
    format!("{:.1}s", started.elapsed().as_secs_f32())
}

fn print_event(started: Instant, event: &EngineEvent) {
    let text = match event {
        EngineEvent::Status(status) => format!("status   {status:?}"),
        EngineEvent::Playing {
            output,
            sample_rate,
        } => format!("playing  to {} at {sample_rate} Hz", output.name),
        EngineEvent::OutputFellBack { from, to } => {
            format!("fallback {from} is gone; now playing to {}", to.name)
        }
        EngineEvent::OutputReturned(device) => format!("returned {} is back", device.name),
        EngineEvent::DefaultMovedOffCable(device) => format!(
            "paused   the default output moved to {}",
            device.as_ref().map_or("nothing", |d| d.name.as_str())
        ),
        EngineEvent::Restored(restored) => format!(
            "restored default output is {}{}",
            restored.device.name,
            if restored.fell_back {
                " (the original was gone)"
            } else {
                ""
            }
        ),
        EngineEvent::Error(message) => format!("ERROR    {message}"),
    };
    println!("{:>7}  {text}", stamp(started));
}

fn report(started: Instant, engine: &Engine) {
    let shared = engine.shared();
    let range = shared
        .take_ring_fill_range()
        .map_or("     -..    -".to_owned(), |(low, high)| {
            format!("{low:>6}..{high:>5}")
        });
    println!(
        "{:>7}  fill [{range}]  in {}  out {}  under {:>3}  over {:>3}  disc {:>3}",
        stamp(started),
        dbfs(shared.take_peak_in()),
        dbfs(shared.take_peak_out()),
        shared.underruns(),
        shared.overruns(),
        shared.discontinuities(),
    );
}

/// Removes `flag <value>` from the arguments and returns the value.
fn take_value(args: &mut Vec<String>, flag: &str) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == flag) else {
        return Ok(None);
    };
    args.remove(index);
    anyhow::ensure!(index < args.len(), "{flag} needs a value");
    Ok(Some(args.remove(index)))
}

/// An output's id from an id or any part of its friendly name.
fn find_output(wanted: &str) -> Result<String> {
    initialize_mta().ok().context("initialising COM (MTA)")?;
    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;
    devices::output_candidates(&enumerator)?
        .into_iter()
        .find(|d| d.id == wanted || d.name.contains(wanted))
        .map(|d| d.id)
        .with_context(|| format!("no real output device matches {wanted:?}"))
}
