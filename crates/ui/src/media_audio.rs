//! Audio extras of the Media pane: equalizer, A-B loop and sleep timer. Everything dispatches
//! engine commands; the widgets only read session state.

use egui::Ui;
use fm_engine::Session;
use fm_media::{BAND_HZ, MAX_BAND_DB, PRESETS};
use serde_json::{Value, json};

type Actions = Vec<(&'static str, Value)>;

/// The submenus for the audio extras, inside the Options menu.
pub(crate) fn menus(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    equalizer(ui, s, actions);
    sleep_menu(ui, s, actions);
    loop_menu(ui, s, actions);
}

fn band_label(hz: f32) -> String {
    if hz >= 1000.0 {
        format!("{}k", hz / 1000.0)
    } else {
        format!("{hz}")
    }
}

fn equalizer(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    let eq = s.media.gain_eq();
    ui.menu_button("Equalizer", |ui| {
        let mut enabled = eq.enabled;
        if ui.checkbox(&mut enabled, "Enabled").changed() {
            actions.push(("media.eq", json!({"enabled": enabled})));
        }
        let label = s
            .media
            .extras
            .eq_preset
            .clone()
            .unwrap_or_else(|| "Custom".into());
        let keep_open = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::IgnoreClicks);
        egui::containers::menu::MenuButton::new(label)
            .config(keep_open)
            .ui(ui, |ui| {
                for (name, _) in PRESETS {
                    let selected = s.media.extras.eq_preset.as_deref() == Some(*name);
                    if ui.selectable_label(selected, *name).clicked() {
                        actions.push(("media.eq", json!({"preset": name})));
                    }
                }
                for preset in &s.media.extras.user_presets {
                    ui.horizontal(|ui| {
                        let selected = s.media.extras.eq_preset.as_deref() == Some(&preset.name);
                        if ui.selectable_label(selected, &preset.name).clicked() {
                            actions.push(("media.eq", json!({"preset": preset.name})));
                        }
                        if ui.small_button("Delete").clicked() {
                            actions.push(("media.eq", json!({"delete": preset.name})));
                        }
                    });
                }
            });
        ui.horizontal(|ui| {
            for (band, hz) in BAND_HZ.iter().enumerate() {
                ui.vertical(|ui| {
                    let mut gain = eq.bands[band];
                    let response = ui.add(
                        egui::Slider::new(&mut gain, -MAX_BAND_DB..=MAX_BAND_DB)
                            .vertical()
                            .step_by(0.5)
                            .show_value(false),
                    );
                    if response.changed() {
                        let mut bands = eq.bands;
                        bands[band] = gain;
                        actions.push(("media.eq", json!({"bands": bands})));
                    }
                    ui.label(band_label(*hz));
                });
            }
        });
        let name_id = ui.make_persistent_id(egui::IdSalt::new("media-eq-save-name"));
        let mut name: String = ui.data_mut(|d| d.get_temp(name_id).unwrap_or_default());
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut name)
                    .hint_text("Preset name")
                    .desired_width(120.0),
            );
            if ui
                .add_enabled(!name.trim().is_empty(), egui::Button::new("Save"))
                .clicked()
            {
                actions.push(("media.eq", json!({"save_as": name.trim()})));
                name.clear();
            }
        });
        ui.data_mut(|d| d.insert_temp(name_id, name));
    });
}

fn loop_menu(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    let extras = &s.media.extras;
    let label = match (extras.loop_a, extras.loop_b) {
        (Some(a), Some(b)) => format!(
            "A-B loop: {} – {}",
            super::media::clock(a),
            super::media::clock(b)
        ),
        (Some(a), None) => format!("A-B loop: from {}", super::media::clock(a)),
        _ => "A-B loop".to_string(),
    };
    ui.menu_button(label, |ui| {
        if ui.button("Set start (A) here").clicked() {
            actions.push(("media.loop", json!({"point": "a"})));
            ui.close();
        }
        if ui
            .add_enabled(
                extras.loop_a.is_some(),
                egui::Button::new("Set end (B) here"),
            )
            .clicked()
        {
            actions.push(("media.loop", json!({"point": "b"})));
            ui.close();
        }
        if ui
            .add_enabled(extras.loop_a.is_some(), egui::Button::new("Clear loop"))
            .clicked()
        {
            actions.push(("media.loop", json!({"point": "clear"})));
            ui.close();
        }
    });
}

fn sleep_menu(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    ui.menu_button("Sleep timer", |ui| {
        for minutes in [15, 30, 45, 60, 90] {
            if ui.button(format!("{minutes} minutes")).clicked() {
                actions.push(("media.sleep", json!({"minutes": minutes})));
                ui.close();
            }
        }
        if ui.button("End of item").clicked() {
            actions.push(("media.sleep", json!({"end_of_item": true})));
            ui.close();
        }
        if ui
            .add_enabled(s.media.extras.sleep.is_some(), egui::Button::new("Cancel"))
            .clicked()
        {
            actions.push(("media.sleep", json!({"cancel": true})));
            ui.close();
        }
    });
}
