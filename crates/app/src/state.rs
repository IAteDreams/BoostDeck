//! App state and the eframe entry point.
//!
//! The UI thread owns this. It talks to the engine's control thread through
//! `EngineCommand`s out and `EngineEvent`s in, and writes the gain straight
//! into the `Shared` atomics so slider moves never wait on a channel.

use std::path::Path;
use std::time::Duration;

use eframe::egui;
use engine::control::{
    DeviceSnapshot, Engine, EngineCommand, EngineEvent, EngineStatus, StartOptions,
};
use engine::devices::DeviceInfo;
use presets::{DeviceRef, PresetStore};
use uuid::Uuid;

use crate::settings::{self, SharedSettings};

/// How often the window redraws while nothing else asks it to: about 30
/// times a second, for the meters.
const REDRAW_EVERY: Duration = Duration::from_millis(33);

/// How fast a meter falls back after a peak, in dB per second. Peaks are
/// read and reset every frame, so without this the bars would flicker.
const METER_FALL_DB_PER_SEC: f32 = 30.0;

/// The bottom of the level meters.
pub const METER_FLOOR_DBFS: f32 = -60.0;

/// Safe start: the most gain a launch starts with. A higher saved gain is
/// offered back with "Restore +X dB?" rather than applied.
pub const SAFE_START_DB: f32 = 6.0;

/// Gains closer than this count as the same, for the modified mark: the
/// slider moves in half-dB steps, so this only absorbs rounding.
const GAIN_EPSILON_DB: f32 = 0.01;

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
    pub presets: PresetStore,
    /// A preset name being typed, or a delete waiting to be confirmed.
    pub preset_edit: Option<PresetEdit>,
    /// Safe start's offer: the saved gain this launch held back.
    pub restore_offer: Option<f32>,
    /// The +24 dB unlock warning is open.
    pub unlock_dialog: bool,
    /// The output the streams last played to this session, to notice a
    /// switch to a new one.
    last_output_id: Option<String>,
    /// An output a preset load just picked: arriving there keeps the
    /// preset's gain instead of resetting to 0 dB.
    preset_output: Option<String>,
}

/// The presets panel's in-progress edits.
#[derive(Debug, Clone)]
pub enum PresetEdit {
    SaveAsNew {
        name: String,
        error: Option<String>,
    },
    Rename {
        id: Uuid,
        name: String,
        error: Option<String>,
    },
    ConfirmDelete(Uuid),
}

impl BoostDeckApp {
    /// Loads the settings and presets, undoes a route a killed run left
    /// behind, starts the engine, and applies the active preset. Boost
    /// itself stays off until the user turns it on.
    pub fn new() -> Self {
        let settings_path = settings::path();
        let settings = SharedSettings::load(settings_path.clone());
        let (presets, presets_loaded) =
            PresetStore::load(settings_path.as_deref().and_then(Path::parent));
        let notice = join_notices(recover_routing(&settings), presets_loaded.notice());

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

        let mut app = Self {
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
            presets,
            preset_edit: None,
            restore_offer: None,
            unlock_dialog: false,
            last_output_id: None,
            preset_output: None,
        };
        app.safe_start();
        app
    }

    /// Starts at the saved gain, but never above `SAFE_START_DB`: a loud
    /// gain left from last time must not blast this launch. A higher one is
    /// offered back instead.
    fn safe_start(&mut self) {
        let (start, offer) = safe_start_gain(self.clamp_gain(self.settings.get().last_gain_db));
        self.set_gain_db(start);
        self.restore_offer = offer;
    }

    /// Answers safe start's offer.
    pub fn answer_restore_offer(&mut self, restore: bool) {
        if let Some(saved) = self.restore_offer.take()
            && restore
        {
            self.set_gain_db(self.clamp_gain(saved));
            self.remember_gain();
        }
    }

    /// The gain as the user sets it by hand: the slider, the panic button.
    /// Any pending offer to restore a louder gain is dropped with it.
    pub fn set_gain_by_hand(&mut self, db: f32) {
        self.restore_offer = None;
        self.set_gain_db(db);
    }

    /// The panic button: 0 dB at once, and remembered.
    pub fn panic_to_zero(&mut self) {
        self.set_gain_by_hand(0.0);
        self.remember_gain();
    }

    /// Saves the gain for safe start. Called when a change settles, not on
    /// every frame of a slider drag.
    pub fn remember_gain(&self) {
        let gain = self.gain_db;
        if let Err(error) = self.settings.update(|s| s.last_gain_db = gain) {
            log::error!("Saving settings failed: {error:#}");
        }
    }

    /// The unlock checkbox. Turning it on only opens the warning; turning it
    /// off takes effect at once and brings a gain above +12 dB back down.
    pub fn set_unlocked(&mut self, unlocked: bool) {
        if unlocked {
            self.unlock_dialog = true;
            return;
        }
        if let Err(error) = self.settings.update(|s| s.unlock_max_boost = false) {
            log::error!("Saving settings failed: {error:#}");
        }
        if self.gain_db > self.clamp_gain(self.gain_db) {
            self.set_gain_db(self.clamp_gain(self.gain_db));
            self.remember_gain();
        }
        self.restore_offer = self.restore_offer.map(|db| self.clamp_gain(db));
    }

    /// The unlock warning's answer.
    pub fn answer_unlock(&mut self, unlock: bool) {
        self.unlock_dialog = false;
        if unlock && let Err(error) = self.settings.update(|s| s.unlock_max_boost = true) {
            log::error!("Saving settings failed: {error:#}");
        }
    }

    pub fn acknowledge_safety_notice(&mut self) {
        if let Err(error) = self.settings.update(|s| s.safety_notice_seen = true) {
            log::error!("Saving settings failed: {error:#}");
        }
    }

    /// Start at 0 dB on a new output: the same boost can be far louder on
    /// headphones than on laptop speakers. A switch a preset load asked for
    /// keeps the preset's gain - choosing it was the point.
    fn arrived_at_output(&mut self, output: &DeviceInfo) {
        let previous = self.last_output_id.replace(output.id.clone());
        let from_preset = self.preset_output.take().as_deref() == Some(output.id.as_str());
        if !resets_for_new_output(previous.as_deref(), &output.id, from_preset, self.gain_db) {
            return;
        }
        self.set_gain_by_hand(0.0);
        self.remember_gain();
        self.notice = Some(format!(
            "Boost was set back to 0 dB for {}. Raise it when you are ready.",
            output.name
        ));
    }

    /// Clamps a gain to the slider's range, which depends on the unlock
    /// setting.
    pub fn clamp_gain(&self, db: f32) -> f32 {
        db.clamp(dsp::GAIN_MIN_DB, self.settings.get().max_gain_db())
    }

    /// Loads a preset: its gain through the smoother - never a jump - and
    /// its output if it names one that is connected. A preset whose output
    /// is unplugged keeps the current one, and says so.
    pub fn load_preset(&mut self, id: Uuid) {
        let preset = match self.presets.set_active(id) {
            Ok(preset) => preset.clone(),
            Err(error) => {
                self.notice = Some(error.to_string());
                return;
            }
        };
        self.set_gain_by_hand(self.clamp_gain(preset.gain_db));
        self.remember_gain();
        let Some(wanted) = preset.output_device else {
            return;
        };
        let found = self
            .devices
            .outputs
            .iter()
            .find(|d| d.id == wanted.id)
            .or_else(|| {
                self.devices
                    .outputs
                    .iter()
                    .find(|d| !wanted.name.is_empty() && d.name == wanted.name)
            })
            .map(|d| d.id.clone());
        match found {
            Some(id) => {
                if self.settings.get().output_device_id.as_deref() != Some(id.as_str()) {
                    self.preset_output = Some(id.clone());
                    self.choose_output(Some(id));
                }
            }
            None => {
                let name = if wanted.name.is_empty() {
                    "Its output device"
                } else {
                    wanted.name.as_str()
                };
                self.notice = Some(format!(
                    "{name} is not connected, so {} kept the current output.",
                    preset.name
                ));
            }
        }
    }

    /// The output as a preset would save it: `None` for "your usual output".
    pub fn current_device_ref(&self) -> Option<DeviceRef> {
        let id = self.settings.get().output_device_id?;
        let name = self
            .devices
            .outputs
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.name.clone())
            .or_else(|| {
                // Unplugged: keep the name the active preset already knows.
                self.presets
                    .active()
                    .and_then(|p| p.output_device.as_ref())
                    .filter(|d| d.id == id)
                    .map(|d| d.name.clone())
            })
            .unwrap_or_default();
        Some(DeviceRef { id, name })
    }

    /// True when the gain differs from the active preset's, or the output
    /// does and the preset names one: the panel then marks it with a dot and
    /// offers Save. A preset with no output - the built-ins - plays wherever
    /// boost is playing, so the output never makes it modified.
    pub fn preset_modified(&self) -> bool {
        let Some(preset) = self.presets.active() else {
            return false;
        };
        let output = self.settings.get().output_device_id;
        (self.gain_db - self.clamp_gain(preset.gain_db)).abs() > GAIN_EPSILON_DB
            || preset
                .output_device
                .as_ref()
                .is_some_and(|d| Some(&d.id) != output.as_ref())
    }

    /// "Save": the current settings into the active preset. One with no
    /// output stays that way; "Save as new" is how to tie one to a device.
    pub fn save_active_preset(&mut self) {
        let Some(preset) = self.presets.active() else {
            return;
        };
        let id = preset.id;
        let output = if preset.output_device.is_some() {
            self.current_device_ref()
        } else {
            None
        };
        if let Err(error) = self.presets.update(id, self.gain_db, output) {
            self.notice = Some(error.to_string());
        }
    }

    /// "Save as new": a new preset from the current settings, made active.
    /// The error is the name's, for the panel to show beside the field.
    pub fn save_preset_as(&mut self, name: &str) -> Result<(), String> {
        let output = self.current_device_ref();
        self.presets
            .create(name, self.gain_db, output)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn rename_preset(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        self.presets.rename(id, name).map_err(|e| e.to_string())
    }

    pub fn duplicate_preset(&mut self, id: Uuid) {
        if let Err(error) = self.presets.duplicate(id) {
            self.notice = Some(error.to_string());
        }
    }

    pub fn delete_preset(&mut self, id: Uuid) {
        if let Err(error) = self.presets.delete(id) {
            self.notice = Some(error.to_string());
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
            } => {
                self.arrived_at_output(&output);
                self.playing = Some((output, sample_rate));
            }
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
        // An unanswered restore offer keeps the louder saved gain for the
        // next launch to offer again, rather than overwriting it with +6 dB.
        if self.restore_offer.is_none() {
            self.remember_gain();
        }
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

/// Safe start: the gain to launch at, and the louder saved gain to offer
/// back, if there is one.
fn safe_start_gain(saved_db: f32) -> (f32, Option<f32>) {
    if saved_db > SAFE_START_DB {
        (SAFE_START_DB, Some(saved_db))
    } else {
        (saved_db, None)
    }
}

/// Whether arriving at `output` resets the boost to 0 dB: only a switch
/// from another output this session, not one a preset load asked for, and
/// only when there is boost to take away.
fn resets_for_new_output(
    previous: Option<&str>,
    output: &str,
    from_preset: bool,
    gain_db: f32,
) -> bool {
    previous.is_some_and(|id| id != output) && !from_preset && gain_db > 0.0
}

/// Both messages, one after the other, or whichever there is.
fn join_notices(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}\n\n{second}")),
        (first, second) => first.or(second),
    }
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

    /// The Safety table: above +6 dB, start at +6 dB and offer the rest.
    #[test]
    fn safe_start_holds_back_a_loud_saved_gain() {
        assert_eq!(safe_start_gain(12.0), (SAFE_START_DB, Some(12.0)));
        assert_eq!(safe_start_gain(24.0), (SAFE_START_DB, Some(24.0)));
        assert_eq!(safe_start_gain(SAFE_START_DB), (SAFE_START_DB, None));
        assert_eq!(safe_start_gain(3.0), (3.0, None));
        assert_eq!(safe_start_gain(-10.0), (-10.0, None));
    }

    /// The Safety table: start at 0 dB on a new output.
    #[test]
    fn only_a_real_switch_to_another_output_resets_the_boost() {
        // Headphones unplugged, playback moved to the speakers.
        assert!(resets_for_new_output(
            Some("headphones"),
            "speakers",
            false,
            6.0
        ));
        // The first start this session, and a rebuild on the same device.
        assert!(!resets_for_new_output(None, "speakers", false, 6.0));
        assert!(!resets_for_new_output(
            Some("speakers"),
            "speakers",
            false,
            6.0
        ));
        // A preset load that picked the output keeps the preset's gain.
        assert!(!resets_for_new_output(
            Some("speakers"),
            "headphones",
            true,
            6.0
        ));
        // Nothing to take away.
        assert!(!resets_for_new_output(
            Some("headphones"),
            "speakers",
            false,
            0.0
        ));
        assert!(!resets_for_new_output(
            Some("headphones"),
            "speakers",
            false,
            -6.0
        ));
    }
}
