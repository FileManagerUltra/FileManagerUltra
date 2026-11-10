//! Transport widgets shared by the Media pane and the Preview pane: the same icon buttons,
//! seek bar and volume control, driven by plain values and command ids.

use egui::{Id, Ui};
use serde_json::{Value, json};

use crate::media_icons::{Icon, bar, button};
use crate::theme::Tokens;

type Actions = Vec<(&'static str, Value)>;

pub(crate) fn clock(seconds: f64) -> String {
    let t = seconds.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

/// Speaker (mute toggle) and a short volume bar.
pub(crate) fn volume(
    ui: &mut Ui,
    tokens: &Tokens,
    volume: f32,
    muted: bool,
    mute_command: &'static str,
    volume_command: &'static str,
    actions: &mut Actions,
) {
    let (icon, label) = if muted {
        (Icon::Muted, "Unmute")
    } else {
        (Icon::Volume, "Mute")
    };
    if button(ui, tokens, icon, label, true, muted).clicked() {
        actions.push((mute_command, json!({"on": !muted})));
    }
    let shown = if muted { 0.0 } else { volume };
    if let Some(change) = bar(
        ui,
        tokens,
        72.0,
        shown / 100.0,
        true,
        "Volume",
        &format!("{volume:.0}%"),
    ) {
        actions.push((volume_command, json!({"value": change.fraction * 100.0})));
    }
}

/// Seek bar; the seek is sent when the drag ends so scrubbing does not flood the decoder.
/// Returns the target in seconds once a drag or click finishes.
pub(crate) fn seek(
    ui: &mut Ui,
    tokens: &Tokens,
    key: Id,
    width: f32,
    position: f64,
    duration: f64,
) -> Option<f64> {
    let dragging: Option<f64> = ui.data(|d| d.get_temp(key));
    let shown = dragging.unwrap_or(position).clamp(0.0, duration.max(0.0));
    let fraction = if duration > 0.0 {
        (shown / duration) as f32
    } else {
        0.0
    };
    match bar(
        ui,
        tokens,
        width,
        fraction,
        duration > 0.0,
        "Seek",
        &clock(shown),
    ) {
        Some(change) if !change.done => {
            ui.data_mut(|d| d.insert_temp(key, f64::from(change.fraction) * duration));
            None
        }
        Some(change) => {
            ui.data_mut(|d| d.remove::<f64>(key));
            Some(f64::from(change.fraction) * duration)
        }
        None => {
            if dragging.is_some() {
                ui.data_mut(|d| d.remove::<f64>(key));
            }
            None
        }
    }
}

/// What a Preview player shows and which commands its controls dispatch.
pub(crate) struct Player {
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub volume: f32,
    pub muted: bool,
    pub play: &'static str,
    pub seek: &'static str,
    pub volume_command: &'static str,
    pub mute: &'static str,
}

/// A framed player card: play/pause, the clock and volume on top, a full-width seek bar below.
pub(crate) fn player_row(
    ui: &mut Ui,
    tokens: &Tokens,
    key: Id,
    player: &Player,
    actions: &mut Actions,
) {
    egui::Frame::new()
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(1.0, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (icon, label) = if player.playing {
                    (Icon::Pause, "Pause")
                } else {
                    (Icon::Play, "Play")
                };
                if button(ui, tokens, icon, label, true, player.playing).clicked() {
                    actions.push((player.play, json!({})));
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} / {}",
                        clock(player.position),
                        clock(player.duration)
                    ))
                    .size(16.0)
                    .monospace(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Laid out right to left: the bar first, then the speaker.
                    let shown = if player.muted { 0.0 } else { player.volume };
                    if let Some(change) = bar(
                        ui,
                        tokens,
                        96.0,
                        shown / 100.0,
                        true,
                        "Volume",
                        &format!("{:.0}%", player.volume),
                    ) {
                        actions.push((
                            player.volume_command,
                            json!({"value": change.fraction * 100.0}),
                        ));
                    }
                    let (icon, label) = if player.muted {
                        (Icon::Muted, "Unmute")
                    } else {
                        (Icon::Volume, "Mute")
                    };
                    if button(ui, tokens, icon, label, true, player.muted).clicked() {
                        actions.push((player.mute, json!({"on": !player.muted})));
                    }
                });
            });
            ui.add_space(4.0);
            let width = ui.available_width();
            if let Some(to) = seek(ui, tokens, key, width, player.position, player.duration) {
                actions.push((player.seek, json!({"to": to})));
            }
        });
    ui.add_space(4.0);
}
