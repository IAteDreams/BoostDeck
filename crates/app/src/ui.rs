//! Window drawing.
//!
//! Top to bottom: status line and boost toggle, any prompt or notice, the
//! dB slider, the three meters, the output picker, the presets panel, and a
//! collapsed settings section. While VB-Cable is not usable the controls give
//! way to the setup screen.

use eframe::egui::{self, Color32, RichText, Sense, Vec2};
use engine::control::{EngineCommand, EngineStatus};

use crate::state::{self, BoostDeckApp, METER_FLOOR_DBFS, PresetEdit, Prompt};

/// Full scale for the gain-reduction meter, in dB. The soft clip keeps the
/// limiter's work to about 1 dB, so 6 dB leaves plenty of headroom.
const GAIN_REDUCTION_SCALE_DB: f32 = 6.0;

const METER_HEIGHT: f32 = 14.0;

const VB_CABLE_URL: &str = "https://vb-audio.com/Cable/";

pub fn draw(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    ui.heading("BoostDeck");
    status_line(app, ui);
    ui.separator();

    safety_notice(app, ui);
    unlock_dialog(app, ui);
    prompt(app, ui);
    notice(app, ui);

    if app.needs_setup() {
        setup_screen(app, ui);
        return;
    }

    gain_slider(app, ui);
    ui.add_space(8.0);
    meters(app, ui);
    ui.add_space(8.0);
    output_picker(app, ui);
    if app.devices.cable_rates_match == Some(false) {
        ui.label(
            RichText::new(
                "CABLE Input and CABLE Output run at different sample rates. \
                 Match them in Sound settings to save a resampling step.",
            )
            .small()
            .weak(),
        );
    }
    ui.add_space(8.0);
    presets_panel(app, ui);
    ui.add_space(8.0);
    settings_section(app, ui);
}

/// The preset dropdown, its five buttons, and whichever edit is open: a name
/// field for Save as new or Rename, or a delete confirmation.
fn presets_panel(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let active = app.presets.active().map(|p| (p.id, p.name.clone()));
    let modified = app.preset_modified();
    let selected_text = match &active {
        Some((_, name)) if modified => format!("{name} \u{2022}"),
        Some((_, name)) => name.clone(),
        None => "(none)".to_owned(),
    };

    let mut load = None;
    ui.horizontal(|ui| {
        ui.label("Preset");
        egui::ComboBox::from_id_salt("preset")
            .selected_text(selected_text)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                for preset in app.presets.list() {
                    let is_active = active.as_ref().is_some_and(|(id, _)| *id == preset.id);
                    let label = format!("{}  ({:+.1} dB)", preset.name, preset.gain_db);
                    // Choosing the active one again reloads it, discarding
                    // changes - the quick way back to a known state.
                    if ui.selectable_label(is_active, label).clicked() {
                        load = Some(preset.id);
                    }
                }
            });
    });
    if let Some(id) = load {
        forget_name_field(ui);
        app.preset_edit = None;
        app.load_preset(id);
    }

    let active_id = active.as_ref().map(|(id, _)| *id);
    let only_one = app.presets.list().len() <= 1;
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(modified, egui::Button::new("Save"))
            .on_hover_text("Overwrite the preset with the current boost and output")
            .clicked()
        {
            app.save_active_preset();
        }
        if ui.button("Save as new").clicked() {
            forget_name_field(ui);
            app.preset_edit = Some(PresetEdit::SaveAsNew {
                name: String::new(),
                error: None,
            });
        }
        if let Some(id) = active_id {
            if ui.button("Rename").clicked() {
                forget_name_field(ui);
                let name = active.as_ref().map(|(_, n)| n.clone()).unwrap_or_default();
                app.preset_edit = Some(PresetEdit::Rename {
                    id,
                    name,
                    error: None,
                });
            }
            if ui.button("Duplicate").clicked() {
                forget_name_field(ui);
                app.preset_edit = None;
                app.duplicate_preset(id);
            }
            if ui
                .add_enabled(!only_one, egui::Button::new("Delete"))
                .on_disabled_hover_text("The last preset cannot be deleted")
                .clicked()
            {
                forget_name_field(ui);
                app.preset_edit = Some(PresetEdit::ConfirmDelete(id));
            }
        }
    });

    preset_edit(app, ui);
}

fn preset_edit(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let Some(edit) = app.preset_edit.take() else {
        return;
    };
    let mut keep = true;
    let edit = match edit {
        PresetEdit::SaveAsNew {
            mut name,
            mut error,
        } => {
            match name_field(ui, "New preset name", &mut name, error.as_deref()) {
                Some(true) => match app.save_preset_as(&name) {
                    Ok(()) => keep = false,
                    Err(message) => error = Some(message),
                },
                Some(false) => keep = false,
                None => {}
            }
            PresetEdit::SaveAsNew { name, error }
        }
        PresetEdit::Rename {
            id,
            mut name,
            mut error,
        } => {
            match name_field(ui, "Rename to", &mut name, error.as_deref()) {
                Some(true) => match app.rename_preset(id, &name) {
                    Ok(()) => keep = false,
                    Err(message) => error = Some(message),
                },
                Some(false) => keep = false,
                None => {}
            }
            PresetEdit::Rename { id, name, error }
        }
        PresetEdit::ConfirmDelete(id) => {
            let name = app
                .presets
                .get(id)
                .map_or_else(String::new, |p| p.name.clone());
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(format!("Delete {name}? This cannot be undone."));
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        app.delete_preset(id);
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            PresetEdit::ConfirmDelete(id)
        }
    };
    if keep {
        app.preset_edit = Some(edit);
    }
}

/// A name field with OK and Cancel. `Some(true)` is OK (or Enter),
/// `Some(false)` Cancel (or Escape), `None` still typing.
fn name_field(
    ui: &mut egui::Ui,
    label: &str,
    name: &mut String,
    error: Option<&str>,
) -> Option<bool> {
    let mut answer = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(label);
        ui.horizontal(|ui| {
            let mut output = egui::TextEdit::singleline(name)
                .char_limit(presets::NAME_LEN_RANGE.1)
                .desired_width(ui.available_width() - 120.0)
                .show(ui);
            // On the frame it opens, take the keyboard and select what is
            // there, so typing replaces the old name rather than needing a
            // click first.
            if !ui.data(|d| d.get_temp::<bool>(name_field_id()).unwrap_or(false)) {
                output.response.request_focus();
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(name.chars().count()),
                );
                output.state.cursor.set_char_range(Some(all));
                output.state.store(ui.ctx(), output.response.id);
                ui.data_mut(|d| d.insert_temp(name_field_id(), true));
            }
            let field = output.response;
            let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("OK").clicked() || entered {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                answer = Some(false);
            }
        });
        if let Some(error) = error {
            ui.label(RichText::new(error).color(Color32::from_rgb(210, 70, 60)));
        }
    });
    if answer.is_some() {
        forget_name_field(ui);
    }
    answer
}

/// Remembers that the open name field has taken focus once.
fn name_field_id() -> egui::Id {
    egui::Id::new("preset-name-field-focused")
}

/// Lets the next name field to open take focus again.
fn forget_name_field(ui: &egui::Ui) {
    ui.data_mut(|d| d.remove::<bool>(name_field_id()));
}

fn status_line(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let mut on = app.boost_on();
        let toggle = ui.add_enabled(
            app.engine.is_some() && !app.needs_setup(),
            egui::Button::new(if on { "Boost on" } else { "Boost off" }).selected(on),
        );
        if toggle.clicked() {
            on = !on;
            if on {
                app.start();
            } else {
                app.send(EngineCommand::Stop);
            }
        }
        let colour = match app.status {
            EngineStatus::Running => Color32::from_rgb(60, 170, 90),
            EngineStatus::Reconnecting { .. } | EngineStatus::PausedOutputChanged => {
                Color32::from_rgb(220, 160, 40)
            }
            EngineStatus::CableMissing => Color32::from_rgb(210, 70, 60),
            EngineStatus::Stopped => ui.visuals().weak_text_color(),
        };
        ui.label(RichText::new(state::status_label(app.status)).color(colour));
    });
    if let Some((output, rate)) = &app.playing {
        ui.label(
            RichText::new(format!("Boosting to {} at {rate} Hz", output.name))
                .small()
                .weak(),
        );
    }
}

/// Shown once, on first launch, over everything else, until acknowledged.
fn safety_notice(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    if app.settings.get().safety_notice_seen {
        return;
    }
    let mut acknowledged = false;
    egui::Modal::new(egui::Id::new("safety-notice")).show(ui.ctx(), |ui| {
        ui.set_max_width(320.0);
        ui.heading("Before you boost");
        ui.label(
            "Boosted audio with heavy limiting can damage small laptop speakers and your \
             hearing if played loud for long periods.",
        );
        ui.add_space(4.0);
        ui.label(format!(
            "Boost is capped at {:+.0} dB, a limiter is always on, and the red 0 dB button \
             turns the boost off at once. Start low, especially on headphones.",
            dsp::GAIN_MAX_DB
        ));
        ui.add_space(8.0);
        acknowledged = ui.button("I understand").clicked();
    });
    // Closing the dialog any other way (Escape, a click outside) does not
    // count: it shows again until acknowledged.
    if acknowledged {
        app.acknowledge_safety_notice();
    }
}

/// The warning behind the +24 dB unlock.
fn unlock_dialog(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    if !app.unlock_dialog {
        return;
    }
    let mut answer = None;
    let modal = egui::Modal::new(egui::Id::new("unlock-boost")).show(ui.ctx(), |ui| {
        ui.set_max_width(320.0);
        ui.heading(format!(
            "Allow boost up to {:+.0} dB?",
            dsp::GAIN_MAX_DB_UNLOCKED
        ));
        ui.label(format!(
            "{:+.0} dB is about 16 times the original level. At that boost most audio is \
             held at the limiter's ceiling, which sounds harsh and can overheat small \
             speakers. Played loud for long, it can damage your hearing.",
            dsp::GAIN_MAX_DB_UNLOCKED
        ));
        ui.add_space(4.0);
        ui.label(
            "Keep the volume low on headphones, and use the red 0 dB button if it is too loud.",
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("Allow {:+.0} dB", dsp::GAIN_MAX_DB_UNLOCKED))
                .clicked()
            {
                answer = Some(true);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(false);
            }
        });
    });
    if modal.should_close() && answer.is_none() {
        answer = Some(false);
    }
    if let Some(unlock) = answer {
        app.answer_unlock(unlock);
    }
}

fn prompt(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let Some(prompt) = &app.prompt else {
        return;
    };
    let (question, yes) = match prompt {
        Prompt::SwitchBack(device) => (
            format!("{} is back. Switch boost to it?", device.name),
            "Switch",
        ),
        Prompt::BoostNewDefault(device) => (
            format!(
                "Boost paused: you made {} the Windows default. Boost it instead?",
                device.name
            ),
            "Boost it",
        ),
    };
    let mut answer = None;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(question);
        ui.horizontal(|ui| {
            if ui.button(yes).clicked() {
                answer = Some(true);
            }
            if ui.button("No").clicked() {
                answer = Some(false);
            }
        });
    });
    if let Some(yes) = answer {
        app.answer_prompt(yes);
    }
    ui.add_space(4.0);
}

fn notice(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let Some(notice) = &app.notice else {
        return;
    };
    let mut dismissed = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(notice);
        dismissed = ui.button("OK").clicked();
    });
    if dismissed {
        app.notice = None;
    }
    ui.add_space(4.0);
}

fn setup_screen(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    ui.heading("VB-Cable is needed");
    if let Some(problem) = &app.devices.cable_problem {
        ui.label(RichText::new(problem).strong());
    }
    ui.label(
        "BoostDeck sends all Windows audio through the free VB-Cable virtual cable, \
         then boosts it on its way to your speakers. Until the cable is ready, your \
         audio plays normally, without boost.",
    );
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label("1. Download VB-CABLE from");
        ui.hyperlink_to("vb-audio.com/Cable", VB_CABLE_URL);
        ui.label("and run the 64-bit setup as administrator, then reboot.");
    });
    ui.label(
        "2. In the classic Sound control panel (mmsys.cpl), note your speakers' \
         sample rate under Properties > Advanced.",
    );
    ui.label("3. Set CABLE Input (Playback) and CABLE Output (Recording) to that same rate.");
    ui.label("4. On CABLE Output > Listen, make sure Listen to this device is off.");
    ui.label("5. On CABLE Input > Advanced, untick Allow applications to take exclusive control.");
    ui.add_space(6.0);
    if ui.button("Check again").clicked() {
        app.send(EngineCommand::RefreshDevices);
    }
}

fn gain_slider(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let max = app.settings.get().max_gain_db();
    let mut gain = app.gain_db.min(max);
    ui.horizontal(|ui| {
        ui.label("Boost");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // The panic button: always visible, one click back to unity.
            let panic = egui::Button::new(RichText::new("0 dB").strong().color(Color32::WHITE))
                .fill(Color32::from_rgb(190, 50, 45));
            if ui
                .add(panic)
                .on_hover_text("Set the boost to 0 dB at once")
                .clicked()
            {
                app.panic_to_zero();
            }
        });
    });
    // Wide enough to set a gain to the half dB; the value box sits beside it.
    ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(120.0);
    let response = ui.add(
        egui::Slider::new(&mut gain, dsp::GAIN_MIN_DB..=max)
            .step_by(0.5)
            .fixed_decimals(1)
            .suffix(" dB"),
    );
    if response.changed() {
        app.set_gain_by_hand(gain);
    }
    // Saved once a change settles: at the end of a drag, or at once for a
    // click or a typed value.
    if response.drag_stopped() || (response.changed() && !response.dragged()) {
        app.remember_gain();
    }
    ui.label(format!(
        "{:+.1} dB = {:.0}% of the original level",
        gain,
        dsp::meter::from_db(gain) * 100.0
    ));

    if let Some(saved) = app.restore_offer {
        let mut answer = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(format!(
                "Started at {:+.0} dB for safety. Last time you were at {saved:+.1} dB.",
                state::SAFE_START_DB
            ));
            ui.horizontal(|ui| {
                if ui.button(format!("Restore {saved:+.1} dB")).clicked() {
                    answer = Some(true);
                }
                if ui.button("Keep this").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(restore) = answer {
            app.answer_restore_offer(restore);
        }
    }
}

fn meters(app: &BoostDeckApp, ui: &mut egui::Ui) {
    let meters = app.meters;
    level_meter(ui, "In", meters.input_dbfs);
    level_meter(ui, "Out", meters.output_dbfs);
    meter_row(
        ui,
        "Limit",
        meters.gain_reduction_db / GAIN_REDUCTION_SCALE_DB,
        Color32::from_rgb(220, 120, 40),
        format!("{:.1} dB", meters.gain_reduction_db),
    );
}

fn level_meter(ui: &mut egui::Ui, name: &str, dbfs: f32) {
    let fraction = (dbfs - METER_FLOOR_DBFS) / -METER_FLOOR_DBFS;
    let colour = if dbfs > dsp::LIMITER_CEILING_DBFS - 0.05 {
        Color32::from_rgb(210, 70, 60)
    } else if dbfs > -12.0 {
        Color32::from_rgb(220, 180, 40)
    } else {
        Color32::from_rgb(60, 170, 90)
    };
    let text = if dbfs <= METER_FLOOR_DBFS {
        "-inf".to_owned()
    } else {
        format!("{dbfs:.1} dBFS")
    };
    meter_row(ui, name, fraction, colour, text);
}

fn meter_row(ui: &mut egui::Ui, name: &str, fraction: f32, colour: Color32, text: String) {
    ui.horizontal(|ui| {
        ui.add_sized([36.0, METER_HEIGHT], egui::Label::new(name));
        let width = (ui.available_width() - 80.0).max(60.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, METER_HEIGHT), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
        let mut filled = rect;
        filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
        painter.rect_filled(filled, 2.0, colour);
        ui.label(RichText::new(text).monospace().small());
    });
}

fn output_picker(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    let chosen = app.settings.get().output_device_id;
    let name_of = |id: &str| {
        app.devices
            .outputs
            .iter()
            .find(|d| d.id == id)
            .map_or("(unplugged device)", |d| d.name.as_str())
            .to_owned()
    };
    let selected_text = match &chosen {
        Some(id) => name_of(id),
        None => "Your usual output".to_owned(),
    };
    let mut picked = chosen.clone();
    ui.horizontal(|ui| {
        ui.label("Output");
        egui::ComboBox::from_id_salt("output")
            .selected_text(selected_text)
            .width(ui.available_width())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut picked, None, "Your usual output")
                    .on_hover_text("Whatever was the Windows default before boost went on");
                for device in &app.devices.outputs {
                    ui.selectable_value(&mut picked, Some(device.id.clone()), &device.name);
                }
            });
    });
    if picked != chosen {
        app.choose_output(picked);
    }
}

fn settings_section(app: &mut BoostDeckApp, ui: &mut egui::Ui) {
    ui.collapsing("Settings", |ui| {
        let mut unlocked = app.settings.get().unlock_max_boost;
        if ui
            .checkbox(
                &mut unlocked,
                format!("Allow boost up to {:+.0} dB", dsp::GAIN_MAX_DB_UNLOCKED),
            )
            .on_hover_text(format!(
                "Normally capped at {:+.0} dB. Turning this on asks first.",
                dsp::GAIN_MAX_DB
            ))
            .changed()
        {
            app.set_unlocked(unlocked);
        }
        ui.add_space(6.0);

        let mut ring_target_ms = app.settings.get().ring_target_ms;
        let (low, high) = dsp::drift::RING_TARGET_MS_RANGE;
        ui.label("Ring target");
        // The boost slider widened the shared spacing; fit this one inside
        // the collapsing section's indent.
        ui.spacing_mut().slider_width = (ui.available_width() - 90.0).max(100.0);
        let response = ui.add(egui::Slider::new(&mut ring_target_ms, low..=high).suffix(" ms"));
        if response.changed()
            && let Err(error) = app.settings.update(|s| s.ring_target_ms = ring_target_ms)
        {
            log::error!("Saving settings failed: {error:#}");
        }
        ui.label(
            RichText::new(
                "Audio kept waiting between the cable and your speakers. Lower means \
                 less delay, higher fewer crackles. Applies the next time boost starts.",
            )
            .small()
            .weak(),
        );

        ui.add_space(6.0);
        ui.label(RichText::new("Debug").strong());
        let Some(engine) = &app.engine else {
            ui.label("The engine is not running.");
            return;
        };
        let shared = engine.shared();
        egui::Grid::new("debug").num_columns(2).show(ui, |ui| {
            ui.label("Underruns");
            ui.label(shared.underruns().to_string());
            ui.end_row();
            ui.label("Overruns");
            ui.label(shared.overruns().to_string());
            ui.end_row();
            ui.label("Discontinuities");
            ui.label(shared.discontinuities().to_string());
            ui.end_row();
            ui.label("Ring fill");
            ui.label(format!(
                "{} frames (average {:.0})",
                shared.ring_fill_frames(),
                shared.ring_fill_average()
            ));
            ui.end_row();
            ui.label("Drift correction");
            ui.label(format!(
                "{} dropped, {} repeated",
                shared.drift_drops(),
                shared.drift_repeats()
            ));
            ui.end_row();
        });
        ui.label(
            RichText::new("Counters restart whenever the streams do.")
                .small()
                .weak(),
        );
    });
}
