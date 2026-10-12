//! BoostDeck desktop app.
//!
//! Coding-order step 1 (Skeleton): this opens the window and nothing else. The
//! audio engine, presets and the real controls arrive in steps 3 through 10.

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

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("BoostDeck")
            .with_inner_size([420.0, 560.0])
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };

    eframe::run_native(
        "BoostDeck",
        options,
        Box::new(|_cc| Ok(Box::<BoostDeckApp>::default())),
    )
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
