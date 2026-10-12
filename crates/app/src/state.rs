//! App state and the eframe entry point.

use crate::settings::Settings;
use eframe::egui;

/// What the status line shows. The engine drives this from step 7; until then
/// it stays `Stopped`.
///
/// The `expect` below starts failing the moment step 7 constructs these, which
/// is the reminder to drop it.
#[expect(dead_code, reason = "driven by the control thread from step 7")]
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Status {
    #[default]
    Stopped,
    Running,
    PausedOutputChanged,
    Reconnecting,
    CableMissing,
}

impl Status {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stopped => "Stopped",
            Self::Running => "Running",
            Self::PausedOutputChanged => "Paused - output changed",
            Self::Reconnecting => "Reconnecting...",
            Self::CableMissing => "Stopped - VB-Cable missing",
        }
    }
}

#[derive(Default)]
pub struct BoostDeckApp {
    pub settings: Settings,
    pub status: Status,
}

impl eframe::App for BoostDeckApp {
    /// eframe 0.36 hands the app a bare `Ui` with no margin or background, so
    /// wrap it in a `CentralPanel` to get the usual padding.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            crate::ui::draw(self, ui);
        });
    }

    /// Step 6 restores the Windows default output device here, alongside the
    /// panic hook, so closing the window never leaves the cable in place.
    fn on_exit(&mut self) {
        log::info!("BoostDeck exiting");
    }
}
