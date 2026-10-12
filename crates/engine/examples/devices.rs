//! Lists every audio endpoint and reports whether VB-Cable is usable.
//!
//! Coding-order step 2. Run it with:
//!
//! ```text
//! cargo run -p engine --example devices
//! cargo run -p engine --example devices -- --set-default Realtek
//! ```
//!
//! It should print an id for CABLE Input, an id for CABLE Output, and an id
//! for your real speakers. That is also the quickest check that a VB-Cable
//! install is visible to WASAPI at all.
//!
//! `--set-default <id or name fragment>` first makes any render device - the
//! cable included - the Console and Multimedia default, the way a user would
//! in Sound settings. It is for testing how BoostDeck reacts to that.

use anyhow::{Context as _, Result};
use engine::devices::{self, CableEndpoints, DeviceInfo};
use wasapi::{DeviceEnumerator, Direction, Role, initialize_mta};

fn main() -> Result<()> {
    initialize_mta().ok().context("initialising COM (MTA)")?;
    let enumerator = DeviceEnumerator::new().context("creating the device enumerator")?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, wanted] = args.as_slice()
        && flag == "--set-default"
    {
        let device = devices::list(&enumerator, Direction::Render)?
            .into_iter()
            .find(|d| d.id == *wanted || d.name.contains(wanted.as_str()))
            .with_context(|| format!("no render device matches {wanted:?}"))?;
        engine::routing::set_default_output(&device.id)?;
        println!("Default output set to {}", device.name);
    }

    for direction in [Direction::Render, Direction::Capture] {
        let devices = devices::list(&enumerator, direction)?;
        println!("\n{direction} devices ({})", devices.len());
        for device in &devices {
            print_device(device);
        }
    }

    println!("\nWindows defaults");
    for direction in [Direction::Render, Direction::Capture] {
        for role in [Role::Console, Role::Multimedia, Role::Communications] {
            let label = format!("  {direction} / {role}");
            match devices::default_device(&enumerator, direction, role)? {
                Some(device) => println!("{label:38} {} [{}]", device.name, device.id),
                None => println!("{label:38} <none>"),
            }
        }
    }

    warn_if_default_is_a_cable(&enumerator)?;
    report_cable(&devices::find_cable(&enumerator)?);

    let candidates = devices::output_candidates(&enumerator)?;
    println!("\nBoostDeck could play to ({})", candidates.len());
    if candidates.is_empty() {
        println!("  <none active>");
    }
    for device in &candidates {
        println!("  {} [{}]", device.name, device.id);
    }

    Ok(())
}

/// BoostDeck points the default output at CABLE Input while boost is on, and
/// puts it back afterwards. Finding a cable there with BoostDeck not running
/// means something else left it that way - a manual change, or a run that did
/// not clean up - and app audio is going into a cable nobody is reading.
fn warn_if_default_is_a_cable(enumerator: &DeviceEnumerator) -> Result<()> {
    for role in [Role::Console, Role::Multimedia] {
        let Some(device) = devices::default_device(enumerator, Direction::Render, role)? else {
            continue;
        };
        if device.is_virtual_cable() {
            println!("\nWARNING: the {role} default output is a virtual cable:");
            println!("  {}", device.name);
            println!("  Nothing reads the other end while BoostDeck is stopped, so apps");
            println!("  playing to it are silent. Set a real output device as the default");
            println!("  in Sound settings to get audio back.");
            return Ok(());
        }
    }
    Ok(())
}

fn print_device(device: &DeviceInfo) {
    let mark = match () {
        _ if device.is_cable_input() => " <- CABLE Input",
        _ if device.is_cable_output() => " <- CABLE Output",
        _ if device.is_virtual_cable() => " <- other virtual cable",
        _ => "",
    };
    println!(
        "  {:<40} {:<10} {:<18} {}{mark}",
        truncate(&device.name, 40),
        device.state.to_string(),
        device.format_label(),
        device.id,
    );
}

fn report_cable(cable: &CableEndpoints) {
    println!("\nVB-Cable");
    match cable.problem() {
        Some(problem) => {
            println!("  NOT READY: {problem}");
            println!("  See the VB-Cable setup steps in the README.");
        }
        None => println!("  ready"),
    }

    if let Some(input) = &cable.input {
        println!("  CABLE Input  {} [{}]", input.format_label(), input.id);
    }
    if let Some(output) = &cable.output {
        println!("  CABLE Output {} [{}]", output.format_label(), output.id);
    }

    // A mismatch still works, since the streams are opened with autoconvert,
    // but it costs an extra resampling step the prerequisites avoid.
    if cable.rates_match() == Some(false) {
        println!("  WARNING: the two sides run at different sample rates.");
        println!("  Set both to your speakers' rate in mmsys.cpl (README step 3).");
    }
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let kept: String = text.chars().take(width.saturating_sub(1)).collect();
    format!("{kept}~")
}
