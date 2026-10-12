//! App state and the eframe entry point.
//!
//! The UI thread owns this. It talks to the engine's control thread through
//! `EngineCommand`s out and `EngineEvent`s in, and writes the gain straight
//! into the `Shared` atomics so slider moves never wait on a channel.

use std::time::Duration;

use eframe::egui;
use engine::control::{
    DeviceSnapshot, Engine, EngineCommand, EngineEvent, EngineStatus, StartOptions,
};
use engine::devices::DeviceInfo;

use crate::settings::{self, SharedSettings};

/// How often the window redraws while nothing else asks it to: about 30
/// times a second, for the meters.
const REDRAW_EVERY: Duration = Duration::from_millis(33);

/// How fast a meter falls back after a peak, in dB per second. Peaks are
/// read and reset every frame, so without this the bars would flicker.
const METER_FALL_DB_PER_SEC: f32 = 30.0;

/// The bottom of the level meters.
pub const METER_FLOOR_DBFS: f32 = -60.0;

/// A question the engine wants put to the user.
#[derive(Debug, Clone)]
pub enum Prompt {
    /// "Your headphones are back. Switch to them?"
    SwitchBack(DeviceInfo),
    /// "You picked another output. Boost that one instead?"
    BoostNewDefault(DeviceInfo),
}

/// Displayed meter values, falling smoothly between peaks.
#[derive(Debug, Clone, Copy)]
pub struct Meters {
    pub input_dbfs: f32,
    pub output_dbfs: f32,
    pub gain_reduction_db: f32,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            input_dbfs: METER_FLOOR_DBFS,
            output_dbfs: METER_FLOOR_DBFS,
            gain_reduction_db: 0.0,
        }
    }
}

pub struct BoostDeckApp {
    pub settings: SharedSettings,
    /// `None` only if the control thread could not be started.
    pub engine: Option<Engine>,
    pub status: EngineStatus,
    /// The output and rate the streams last started on, while running.
    pub playing: Option<(DeviceInfo, u32)>,
    pub devices: DeviceSnapshot,
    /// False until the engine's first device list arrives, so the setup
    /// screen does not flash up before anything is known.
    pub devices_known: bool,
    pub gain_db: f32,
    pub meters: Meters,
    pub prompt: Option<Prompt>,
    /// A message for the user that stays until dismissed.
    pub notice: Option<String>,
}

impl BoostDeckApp {
    /// Loads the settings, undoes a route a killed run left behind, and
    /// starts the engine. Boost itself stays off until the user turns it on.
    pub fn new() -> Self {
        let settings = SharedSettings::load(settings::path());
        let notice = recover_routing(&settings);

        let engine = match Engine::spawn(Box::new(settings.clone())) {
            Ok(engine) => Some(engine),
            Err(error) => {
                log::error!("Starting the engine failed: {error:#}");
                None
            }
        };
        let notice = match &engine {
            Some(_) => notice,
            None => Some("The audio engine could not start; see the log.".to_owned()),
        };

        Self {
            settings,
            engine,
            status: EngineStatus::Stopped,
            playing: None,
            devices: DeviceSnapshot::default(),
            devices_known: false,
            gain_db: 0.0,
            meters: Meters::default(),
            prompt: None,
            notice,
        }
    }

    /// True while boost is on, including while it reconnects.
    pub fn boost_on(&self) -> bool {
        matches!(
            self.status,
            EngineStatus::Running | EngineStatus::Reconnecting { .. }
        )
    }

    /// The setup screen shows while VB-Cable is not usable.
    pub fn needs_setup(&self) -> bool {
        self.devices_known && self.devices.cable_problem.is_some()
    }

    pub fn send(&self, command: EngineCommand) {
        if let Some(engine) = &self.engine {
            engine.send(command);
        }
    }

    /// Turns boost on with the chosen output and ring target, or, while it
    /// is already on, moves it to them.
    pub fn start(&self) {
        let settings = self.settings.get();
        // A device picked once and since unplugged falls back to the
        // original default rather than refusing to start.
        let output_id = settings
            .output_device_id
            .filter(|id| self.devices.outputs.iter().any(|d| &d.id == id));
        self.send(EngineCommand::Start(StartOptions {
            output_id,
            ring_target_ms: settings.ring_target_ms,
        }));
    }

    pub fn set_gain_db(&mut self, db: f32) {
        self.gain_db = db;
        if let Some(engine) = &self.engine {
            engine.shared().set_target_gain_db(db);
        }
    }

    /// Picks the output for boost, remembers it, and moves to it at once if
    /// boost is on.
    pub fn choose_output(&mut self, id: Option<String>) {
        if let Err(error) = self.settings.update(|s| s.output_device_id = id) {
            log::error!("Saving settings failed: {error:#}");
        }
        if self.boost_on() {
            self.start();
        }
    }

    pub fn answer_prompt(&mut self, yes: bool) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        if !yes {
            return;
        }
        match prompt {
            Prompt::SwitchBack(device) => self.send(EngineCommand::SetOutput(device.id)),
            Prompt::BoostNewDefault(device) => self.choose_output_and_start(device.id),
        }
    }

    fn choose_output_and_start(&mut self, id: String) {
        if let Err(error) = self.settings.update(|s| s.output_device_id = Some(id)) {
            log::error!("Saving settings failed: {error:#}");
        }
        self.start();
    }

    /// Drains the engine's events and refreshes the meters. Once a frame.
    fn poll_engine(&mut self, frame_secs: f32) {
        let Some(engine) = &self.engine else {
            return;
        };
        let events: Vec<EngineEvent> = engine.events().try_iter().collect();
        let shared = engine.shared();
        let fall = METER_FALL_DB_PER_SEC * frame_secs;
        self.meters.input_dbfs = falling(self.meters.input_dbfs, dbfs(shared.take_peak_in()), fall);
        self.meters.output_dbfs =
            falling(self.meters.output_dbfs, dbfs(shared.take_peak_out()), fall);
        self.meters.gain_reduction_db = shared
            .take_gain_reduction_db()
            .max(self.meters.gain_reduction_db - fall)
            .max(0.0);

        for event in events {
            self.apply(event);
        }
    }

    fn apply(&mut self, event: EngineEvent) {
        log::debug!("Engine event: {event:?}");
        match event {
            EngineEvent::Status(status) => {
                self.status = status;
                if !self.boost_on() {
                    self.playing = None;
                }
            }
            EngineEvent::Playing {
                output,
                sample_rate,
            } => self.playing = Some((output, sample_rate)),
            EngineEvent::OutputFellBack { from, to } => {
                self.notice = Some(format!("{from} went away; now playing to {}.", to.name));
            }
            EngineEvent::OutputReturned(device) => self.prompt = Some(Prompt::SwitchBack(device)),
            EngineEvent::DefaultMovedOffCable(Some(device)) if !device.is_virtual_cable() => {
                self.prompt = Some(Prompt::BoostNewDefault(device));
            }
            EngineEvent::DefaultMovedOffCable(_) => {
                self.notice = Some(
                    "Boost paused: the Windows default output was changed. \
                     Turn boost on again to resume."
                        .to_owned(),
                );
            }
            EngineEvent::Restored(restored) => {
                self.playing = None;
                if restored.fell_back {
                    self.notice = Some(format!(
                        "Your original output is gone, so the default is now {}.",
                        restored.device.name
                    ));
                }
            }
            EngineEvent::Devices(snapshot) => {
                self.devices = snapshot;
                self.devices_known = true;
            }
            EngineEvent::Error(message) => self.notice = Some(message),
        }
    }
}

impl eframe::App for BoostDeckApp {
    /// eframe 0.36 hands the app a bare `Ui` with no margin or background, so
    /// wrap it in a `CentralPanel` to get the usual padding.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Ctrl+C in the debug console closes the window the normal way, so
        // `on_exit` restores the default output.
        if engine::console::interrupted() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        let frame_secs = ui.ctx().input(|i| i.stable_dt).min(0.1);
        self.poll_engine(frame_secs);

        egui::CentralPanel::default().show(ui, |ui| {
            crate::ui::draw(self, ui);
        });
        ui.ctx().request_repaint_after(REDRAW_EVERY);
    }

    /// Closing the window never leaves the cable in place: dropping the
    /// engine stops it, which restores the default. The panic hook covers
    /// crashes, and the launch check covers a kill from Task Manager.
    fn on_exit(&mut self) {
        drop(self.engine.take());
        if self.settings.get().routing_active
            && let Err(error) = end_routing(&self.settings)
        {
            log::error!("Restoring the default output on exit failed: {error:#}");
        }
        log::info!("BoostDeck exiting");
    }
}

/// The launch check: `routing_active` still set means the last run was
/// killed or crashed while the cable was the default. Returns what to tell
/// the user, if anything happened.
fn recover_routing(settings: &SharedSettings) -> Option<String> {
    if !settings.get().routing_active {
        return None;
    }
    log::warn!("routing_active was left set: the last run did not clean up");
    Some(match end_routing(settings) {
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

/// Restores the original default output and clears the routing record. On
/// failure the record is kept, so the next launch tries again.
fn end_routing(settings: &SharedSettings) -> anyhow::Result<engine::routing::Restored> {
    let original = settings.get().original_default_device_id;
    let restored = engine::routing::restore_on_own_thread(original)?;
    settings.update(|s| {
        s.routing_active = false;
        s.original_default_device_id = None;
    })?;
    Ok(restored)
}

fn dbfs(peak: f32) -> f32 {
    if peak > 0.0 {
        dsp::meter::to_db(peak).max(METER_FLOOR_DBFS)
    } else {
        METER_FLOOR_DBFS
    }
}

/// A meter that jumps up to a new peak and falls back slowly.
fn falling(shown: f32, peak: f32, fall: f32) -> f32 {
    peak.max(shown - fall).max(METER_FLOOR_DBFS)
}

pub fn status_label(status: EngineStatus) -> String {
    match status {
        EngineStatus::Stopped => "Stopped".to_owned(),
        EngineStatus::Running => "Running".to_owned(),
        EngineStatus::Reconnecting { attempt } => format!("Reconnecting... (attempt {attempt})"),
        EngineStatus::PausedOutputChanged => "Paused - output changed".to_owned(),
        EngineStatus::CableMissing => "Stopped - VB-Cable missing".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_jumps_up_and_falls_back_slowly() {
        assert_eq!(falling(-40.0, -6.0, 1.0), -6.0);
        assert_eq!(falling(-6.0, -40.0, 1.0), -7.0);
        assert_eq!(falling(-59.5, METER_FLOOR_DBFS, 1.0), METER_FLOOR_DBFS);
    }

    #[test]
    fn silence_reads_as_the_floor() {
        assert_eq!(dbfs(0.0), METER_FLOOR_DBFS);
        assert!((dbfs(1.0) - 0.0).abs() < 1e-4);
    }
}
