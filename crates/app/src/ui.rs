//! Window drawing.
//!
//! Step 8 builds the real layout: status line and boost toggle, the dB slider,
//! the three meters, the output picker, the presets panel, and a collapsed
//! settings section. For now this shows where the app stands.

use crate::state::BoostDeckApp;
use eframe::egui;

pub fn draw(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    ui.heading("BoostDeck");
    ui.label(format!("Status: {}", app.status.label()));

    ui.separator();

    ui.label("Skeleton build - the audio path is not wired up yet.");
    ui.label(format!(
        "Boost will clamp at {:+.0} dB, limiter fixed at {:+.0} dBFS.",
        app.settings.max_gain_db(),
        dsp::LIMITER_CEILING_DBFS,
    ));
    ui.label(format!(
        "Ring target {} ms of the {}-{} ms range.",
        app.settings.ring_target_ms,
        dsp::drift::RING_TARGET_MS_RANGE.0,
        dsp::drift::RING_TARGET_MS_RANGE.1,
    ));

    ui.separator();

    ui.label("Next: step 2, list the audio devices and find VB-Cable.");
}
