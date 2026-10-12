//! BoostDeck desktop app.
//!
//! Opens the window and starts the audio engine behind it. On launch it first
//! undoes a default-output switch left behind by a run that was killed or
//! crashed. Presets and the safety rules arrive in steps 9 and 10.

// No console window in release builds (README "Packaging"), but keep one in
// debug builds so `log` output and panics stay visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;
mod state;
mod ui;

use state::BoostDeckApp;

fn main() -> eframe::Result {
    init_logging();
    log::info!("BoostDeck {} starting", env!("CARGO_PKG_VERSION"));
    // Before anything can route: a panic while the cable is the default puts
    // the user's device back on the way down.
    engine::routing::install_panic_hook();
    // Ctrl+C in the debug console would otherwise end the process with no
    // `on_exit`, leaving the cable as the default.
    if let Err(error) = engine::console::catch_ctrl_c() {
        log::warn!("{error:#}");
    }
    // Built before the window opens, so a leftover route from a killed run
    // is undone even if the window never appears.
    let app = BoostDeckApp::new();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("BoostDeck")
            .with_inner_size([420.0, 560.0])
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native("BoostDeck", options, Box::new(|_cc| Ok(Box::new(app))))
}

/// Logging goes to the terminal for now. Step 11 moves it to a rolling file
/// under `%LOCALAPPDATA%\BoostDeck\logs`, keeping the last 5.
///
/// Release builds have no console attached, so failing to install the logger
/// is expected and must not stop the app.
fn init_logging() {
    let _ = simplelog::TermLogger::init(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto,
    );
}
