//! BoostDeck desktop app.
//!
//! Opens the window and starts the audio engine behind it. On launch it first
//! undoes a default-output switch left behind by a run that was killed or
//! crashed.

// No console window in release builds (README "Packaging"), but keep one in
// debug builds so `log` output and panics stay visible while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod logging;
mod settings;
mod state;
mod ui;

use std::sync::Arc;

use state::BoostDeckApp;

/// The window and taskbar icon. The `.exe` itself carries the `.ico`, from
/// `build.rs`.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/boostdeck-256.png");

fn main() -> eframe::Result {
    let log_file = logging::init();
    log::info!("BoostDeck {} starting", env!("CARGO_PKG_VERSION"));
    if let Some(path) = log_file {
        log::info!("Logging to {}", path.display());
    }
    // Before anything can route: a panic while the cable is the default puts
    // the user's device back on the way down. Chained after the panic
    // logger, so the restore happens first and the panic is still logged.
    engine::routing::install_panic_hook();
    // Ctrl+C in the debug console would otherwise end the process with no
    // `on_exit`, leaving the cable as the default.
    if let Err(error) = engine::console::catch_ctrl_c() {
        log::warn!("{error:#}");
    }
    // Built before the window opens, so a leftover route from a killed run
    // is undone even if the window never appears.
    let app = BoostDeckApp::new();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("BoostDeck")
        .with_inner_size([420.0, 560.0])
        .with_min_inner_size([360.0, 420.0]);
    match eframe::icon_data::from_png_bytes(ICON_PNG) {
        Ok(icon) => viewport = viewport.with_icon(Arc::new(icon)),
        Err(error) => log::warn!("The window icon did not load: {error}"),
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native("BoostDeck", options, Box::new(|_cc| Ok(Box::new(app))))
}
