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
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
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
            let preset_label = ui.weak("Preset");
            let combo = egui::ComboBox::from_id_salt("eq_preset")
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
            combo.response.labelled_by(preset_label.id);
        });
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.vertical(|ui| {
                ui.set_height(BAND_HEIGHT + 24.0);
                ui.add_space(24.0);
                ui.small(format!("+{MAX_BAND_DB:.0} dB"));
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    ui.small(format!("-{MAX_BAND_DB:.0} dB"));
                });
            });
            for (band, hz) in BAND_HZ.iter().enumerate() {
                ui.allocate_ui_with_layout(
                    egui::vec2(34.0, 0.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let band_name = ui.label(band_label(*hz));
                        let gain = eq.bands[band];
                        if let Some(new) = vertical_slider(ui, gain, &BAND, BAND_HEIGHT) {
                            let mut bands = eq.bands;
                            bands[band] = new;
                            actions.push(("media.eq", json!({"bands": bands})));
                        }
                        ui.small(format!("{gain:+.1}"));
                        let _ = band_name;
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

const BAND_HEIGHT: f32 = 130.0;

/// Range, rest point (where the fill starts), step and names of a vertical slider.
pub(crate) struct Scale {
    pub min: f32,
    pub max: f32,
    pub origin: f32,
    pub step: f32,
    pub name: &'static str,
    pub unit: &'static str,
}

const BAND: Scale = Scale {
    min: -MAX_BAND_DB,
    max: MAX_BAND_DB,
    origin: 0.0,
    step: 0.5,
    name: "Equalizer band",
    unit: " dB",
};

/// A vertical slider: a track with a rest line, filled from the rest point to the knob.
pub(crate) fn vertical_slider(ui: &mut Ui, value: f32, scale: &Scale, height: f32) -> Option<f32> {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(24.0, height), egui::Sense::click_and_drag());
    let visuals = ui.visuals().clone();
    let track = egui::Rect::from_center_size(rect.center(), egui::vec2(4.0, rect.height()));
    let span = track.height() - 12.0;
    let range = scale.max - scale.min;
    let y_of = |v: f32| track.max.y - 6.0 - (v - scale.min) / range * span;
    let painter = ui.painter();
    painter.rect_filled(
        track,
        2.0,
        visuals
            .widgets
            .noninteractive
            .fg_stroke
            .color
            .gamma_multiply(0.3),
    );
    painter.hline(
        rect.x_range(),
        y_of(scale.origin),
        egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color),
    );
    let knob = egui::pos2(track.center().x, y_of(value));
    painter.rect_filled(
        egui::Rect::from_two_pos(
            egui::pos2(track.min.x, y_of(scale.origin)),
            egui::pos2(track.max.x, knob.y),
        ),
        2.0,
        visuals.selection.bg_fill,
    );
    let big = response.hovered() || response.dragged();
    painter.circle_filled(
        knob,
        if big { 7.0 } else { 6.0 },
        visuals.selection.stroke.color,
    );
    response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(value), scale.name));
    let changed = if response.dragged() || response.clicked() {
        response.interact_pointer_pos().map(|pos| {
            let v = scale.min + (track.max.y - 6.0 - pos.y) / span * range;
            ((v / scale.step).round() * scale.step).clamp(scale.min, scale.max)
        })
    } else {
        None
    };
    response.on_hover_text(match scale.unit {
        " dB" => format!("{value:+.1} dB"),
        unit => format!("{value:.0}{unit}"),
    });
    changed
}
