//! App state and the eframe entry point.

use std::path::PathBuf;

use crate::settings::{self, Settings};
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
    /// Where `settings` is saved, if Windows gave us an app-data folder.
    pub settings_path: Option<PathBuf>,
    pub status: Status,
    /// A message for the user that stays until dismissed, such as the
    /// launch-time recovery report.
    pub notice: Option<String>,
}

impl BoostDeckApp {
    /// Loads the settings and, if the last run left the cable as the default
    /// output, puts the user's device back before anything else happens.
    pub fn new() -> Self {
        let settings_path = settings::path();
        let settings = settings_path
            .as_deref()
            .map(Settings::load)
            .unwrap_or_default();
        let mut app = Self {
            settings,
            settings_path,
            ..Self::default()
        };
        app.notice = app.recover_routing();
        app
    }

    /// The launch check: `routing_active` still set means the last run was
    /// killed or crashed while the cable was the default. Returns what to
    /// tell the user, if anything happened.
    fn recover_routing(&mut self) -> Option<String> {
        if !self.settings.routing_active {
            return None;
        }
        log::warn!("routing_active was left set: the last run did not clean up");
        Some(match self.end_routing() {
            Ok(restored) if restored.fell_back => format!(
                "BoostDeck did not shut down cleanly last time and left VB-Cable as the \
                 default output. Your original device is gone, so the default is now {}.",
                restored.device.name
            ),
            Ok(restored) => format!(
                "BoostDeck did not shut down cleanly last time and left VB-Cable as the \
                 default output. It has been restored to {}.",
                restored.device.name
            ),
            Err(error) => format!(
                "BoostDeck did not shut down cleanly last time and could not restore your \
                 default output: {error:#}. Set your speakers as the default in Sound settings."
            ),
        })
    }

    /// Restores the original default output and clears the routing record.
    /// On failure the record is kept, so the next launch tries again.
    fn end_routing(&mut self) -> anyhow::Result<engine::routing::Restored> {
        let restored = engine::routing::restore_on_own_thread(
            self.settings.original_default_device_id.clone(),
        )?;
        self.settings.routing_active = false;
        self.settings.original_default_device_id = None;
        self.save_settings();
        Ok(restored)
    }

    fn save_settings(&self) {
        let Some(path) = &self.settings_path else {
            log::warn!("No app-data folder; settings not saved");
            return;
        };
        if let Err(error) = self.settings.save(path) {
            log::error!("Saving settings failed: {error:#}");
        }
    }
}

impl eframe::App for BoostDeckApp {
    /// eframe 0.36 hands the app a bare `Ui` with no margin or background, so
    /// wrap it in a `CentralPanel` to get the usual padding.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            crate::ui::draw(self, ui);
        });
    }

    /// Closing the window never leaves the cable in place. The panic hook
    /// covers crashes, and the launch check covers a kill from Task Manager.
    fn on_exit(&mut self) {
        if self.settings.routing_active
            && let Err(error) = self.end_routing()
        {
            log::error!("Restoring the default output on exit failed: {error:#}");
        }
        log::info!("BoostDeck exiting");
    }
}
