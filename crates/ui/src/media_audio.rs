//! Audio extras of the Media pane: equalizer, A-B loop and sleep timer. Everything dispatches
//! engine commands; the widgets only read session state.

use egui::Ui;
use fm_engine::Session;
use fm_media::{BAND_HZ, MAX_BAND_DB, PRESETS};
use serde_json::{Value, json};

type Actions = Vec<(&'static str, Value)>;

/// The submenus for the audio extras, inside the Options menu.
pub(crate) fn menus(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    if ui.button("Equalizer…").clicked() {
        let id = eq_open_id(ui);
        ui.data_mut(|d| d.insert_temp(id, true));
        ui.close();
    }
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

/// Global, because the menu and the window are built from different `Ui` scopes.
fn eq_open_id(_ui: &Ui) -> egui::Id {
    egui::Id::NULL.with("media-eq-window-open")
}

/// The equalizer lives in its own window so picking a preset never dismisses it.
pub(crate) fn equalizer_window(ui: &Ui, s: &Session, actions: &mut Actions) {
    let id = eq_open_id(ui);
    let mut open: bool = ui.data(|d| d.get_temp(id).unwrap_or(false));
    if !open {
        return;
    }
    egui::Window::new("Equalizer")
        .id(egui::Id::NULL.with("media-eq-window"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ui.ctx().content_rect().center())
        .show(ui.ctx(), |ui| equalizer(ui, s, actions));
    ui.data_mut(|d| d.insert_temp(id, open));
}

fn equalizer(ui: &mut Ui, s: &Session, actions: &mut Actions) {
    let eq = s.media.gain_eq();
    {
        ui.add_space(4.0);
        let label = s
            .media
            .extras
            .eq_preset
            .clone()
            .unwrap_or_else(|| "Custom".into());
        ui.horizontal(|ui| {
            let mut enabled = eq.enabled;
            if ui.toggle_value(&mut enabled, "Enabled").changed() {
                actions.push(("media.eq", json!({"enabled": enabled})));
            }
            ui.add_space(8.0);
            ui.weak("Preset");
            egui::ComboBox::from_id_salt("eq_preset")
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for (name, _) in PRESETS {
                        let selected = s.media.extras.eq_preset.as_deref() == Some(*name);
                        if ui.selectable_label(selected, *name).clicked() {
                            actions.push(("media.eq", json!({"preset": name})));
                        }
                    }
                    for preset in &s.media.extras.user_presets {
                        ui.horizontal(|ui| {
                            let selected =
                                s.media.extras.eq_preset.as_deref() == Some(&preset.name);
                            if ui.selectable_label(selected, &preset.name).clicked() {
                                actions.push(("media.eq", json!({"preset": preset.name})));
                            }
                            if ui.small_button("Delete").clicked() {
                                actions.push(("media.eq", json!({"delete": preset.name})));
                            }
                        });
                    }
                });
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        let accent = ui.visuals().selection.bg_fill;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.vertical(|ui| {
                ui.set_height(150.0);
                ui.label(format!("+{MAX_BAND_DB:.0} dB"));
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.label(format!("-{MAX_BAND_DB:.0} dB"));
                });
            });
            for (band, hz) in BAND_HZ.iter().enumerate() {
                ui.allocate_ui_with_layout(
                    egui::vec2(36.0, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let mut gain = eq.bands[band];
                        ui.spacing_mut().slider_width = 150.0;
                        ui.visuals_mut().selection.bg_fill = accent;
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
                        ui.small(format!("{gain:+.1}"));
                    },
                );
            }
        });
        ui.add_space(6.0);
        ui.separator();
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
    }
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
