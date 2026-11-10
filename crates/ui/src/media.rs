//! Media is a view of session playback; all controls dispatch engine commands.

use egui::{Id, ScrollArea, Ui};
use fm_engine::Session;
use fm_layout::PaneId;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::theme::Tokens;

/// Controls and the mouse cursor hide after this long without input in fullscreen.
pub const HIDE_AFTER: f64 = 3.0;
/// A single click toggles playback only after this long, so a double click can
/// enter or leave fullscreen without pausing.
pub const CLICK_DELAY: f64 = 0.3;

/// Whether the fullscreen controls show after `idle` seconds without input.
pub fn controls_visible(idle: f64) -> bool {
    idle < HIDE_AFTER
}

pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    if s.media_fullscreen_active() && s.media.is_video() {
        fullscreen(ui, s, pane, tokens);
        return;
    }
    let mut actions: Vec<(&str, Value)> = Vec::new();
    let mut slider_focus = false;
    let media = &s.media;
    let current = media.queue.current_index();
    let title = media
        .info
        .as_ref()
        .and_then(|info| info.tags.title.as_deref())
        .or_else(|| media.queue.current().and_then(|uri| uri.file_name()))
        .unwrap_or("No media selected");
    if media.is_video() {
        let height = (ui.available_height() * 0.5).clamp(64.0, 360.0);
        video(
            ui,
            s,
            pane,
            egui::vec2(ui.available_width(), height),
            tokens,
            &mut actions,
        );
        ui.heading(title);
    } else {
        ui.horizontal(|ui| {
            cover(ui, pane, media.cover.as_ref(), tokens);
            ui.vertical(|ui| {
                ui.heading(title);
                if let Some(info) = &media.info {
                    for value in [&info.tags.artist, &info.tags.album].into_iter().flatten() {
                        ui.label(tokens.muted(value));
                    }
                }
            });
        });
    }
    ui.horizontal_wrapped(|ui| {
        for (label, command) in [
            ("Previous", "media.prev"),
            (if media.playing { "Pause" } else { "Play" }, "media.toggle"),
            ("Stop", "media.stop"),
            ("Next", "media.next"),
        ] {
            if ui
                .add_enabled(s.command_enabled(command), egui::Button::new(label))
                .clicked()
            {
                actions.push((command, Value::Null));
            }
        }
        if media.is_video()
            && ui
                .add_enabled(
                    s.command_enabled("media.fullscreen"),
                    egui::Button::new("Fullscreen"),
                )
                .on_hover_text("Fullscreen (F)")
                .clicked()
        {
            actions.push(("media.fullscreen", Value::Null));
        }
        track_menus(ui, s, &mut actions);
        ui.menu_button("Keys", keys_help);
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().slider_width = 140.0;
        slider_focus |= seek_slider(ui, s, pane, tokens, &mut actions);
    });
    if let Some(video) = &media.video {
        ui.horizontal(|ui| {
            for (label, frames) in [("Previous frame", -1), ("Next frame", 1)] {
                if ui
                    .add_enabled(s.command_enabled("media.step"), egui::Button::new(label))
                    .clicked()
                {
                    actions.push(("media.step", json!({"frames": frames})));
                }
            }
        });
        ui.horizontal(|ui| {
            let mut speed = media.speed as f32;
            let response = ui.add(
                egui::Slider::new(&mut speed, 0.5..=3.0)
                    .step_by(0.05)
                    .text("Speed"),
            );
            slider_focus |= response.has_focus();
            if response.changed() {
                actions.push(("media.speed", json!({"value": speed})));
            }
        });
        if let Some(hint) = &video.hint {
            crate::dialog_style::hint(ui, tokens, hint);
        }
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().slider_width = 140.0;
        slider_focus |= volume_slider(ui, s, &mut actions);
        let mut muted = media.muted;
        if ui.checkbox(&mut muted, "Mute").changed() {
            actions.push(("media.mute", json!({"on": muted})));
        }
        let mut shuffle = media.queue.shuffle();
        if ui.checkbox(&mut shuffle, "Shuffle").changed() {
            actions.push(("media.shuffle", json!({"on": shuffle})));
        }
        ui.menu_button(format!("Repeat: {:?}", media.queue.repeat()), |ui| {
            for mode in ["off", "all", "one"] {
                if ui.button(mode).clicked() {
                    actions.push(("media.repeat", json!({"mode": mode})));
                    ui.close();
                }
            }
        });
        if ui
            .add_enabled(
                !media.queue.items().is_empty(),
                egui::Button::new("Clear queue"),
            )
            .clicked()
        {
            actions.push(("media.clear", Value::Null));
        }
    });
    crate::media_audio::controls(ui, s, &mut actions);
    let mut gapless = s.prefs.media_gapless;
    if ui
        .checkbox(&mut gapless, "Gapless playback")
        .on_hover_text("Join consecutive local tracks without a pause")
        .changed()
    {
        actions.push((
            "prefs.set",
            json!({"key": "media_gapless", "value": gapless}),
        ));
    }
    let mut background = s.prefs.media_background_audio;
    if ui
        .checkbox(
            &mut background,
            "Keep audio playing when the Media pane is closed",
        )
        .changed()
    {
        actions.push((
            "prefs.set",
            json!({"key": "media_background_audio", "value": background}),
        ));
    }
    if let Some(error) = &media.error {
        ui.colored_label(tokens.error, error);
    }
    if let Some((done, total)) = media.spool_progress() {
        let bar = if total > 0 {
            egui::ProgressBar::new((done as f64 / total as f64) as f32)
        } else {
            egui::ProgressBar::new(0.0).animate(true)
        };
        ui.add(bar.text(spool_label(done, total)));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    ui.separator();
    if media.queue.items().is_empty() {
        ui.label(tokens.muted(
            "Select audio or video files in Files and choose Media > Play or Add to queue.",
        ));
    }
    let selected = s.layout.state(pane).and_then(|state| state.media.selected);
    let scroll = s
        .layout
        .state(pane)
        .map_or(0.0, |state| state.media.queue_scroll);
    let mut selection = selected;
    let output = ScrollArea::vertical()
        .id_salt(("media-queue", pane))
        .vertical_scroll_offset(scroll)
        .auto_shrink(false)
        .show_rows(
            ui,
            tokens.row_height,
            media.queue.items().len(),
            |ui, range| {
                for index in range {
                    let Some(uri) = media.queue.items().get(index) else {
                        continue;
                    };
                    ui.horizontal(|ui| {
                        let name = uri.file_name().unwrap_or("Media");
                        let text = if Some(index) == current {
                            egui::RichText::new(format!("> {name}"))
                                .color(tokens.text)
                                .strong()
                        } else {
                            egui::RichText::new(name).color(tokens.text)
                        };
                        let row = ui.selectable_label(selection == Some(index), text);
                        if row.clicked() {
                            selection = Some(index);
                        }
                        if row.double_clicked() {
                            actions.push(("media.play", json!({"index": index})));
                        }
                        row.context_menu(|ui| {
                            if ui.button("Play").clicked() {
                                actions.push(("media.play", json!({"index": index})));
                                ui.close();
                            }
                            if ui.button("Remove").clicked() {
                                actions.push(("media.remove", json!({"index": index})));
                                ui.close();
                            }
                            if ui
                                .add_enabled(index > 0, egui::Button::new("Move up"))
                                .clicked()
                            {
                                actions.push((
                                    "media.move",
                                    json!({"from": index, "to": index.saturating_sub(1)}),
                                ));
                                ui.close();
                            }
                            if ui
                                .add_enabled(
                                    index + 1 < media.queue.items().len(),
                                    egui::Button::new("Move down"),
                                )
                                .clicked()
                            {
                                actions
                                    .push(("media.move", json!({"from": index, "to": index + 1})));
                                ui.close();
                            }
                        });
                    });
                }
            },
        );
    if let Some(state) = s.layout.state_mut(pane) {
        state.media.selected = selection;
        state.media.queue_scroll = output.state.clamped_offset().y;
    }
    crate::media_keys::note_slider_focus(ui.ctx(), slider_focus);
    for (command, params) in actions {
        s.dispatch_or_notice(command, &params);
    }
}

/// Seek bar with the clock; returns whether the slider has keyboard focus.
fn seek_slider(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<(&'static str, Value)>,
) -> bool {
    let media = &s.media;
    let duration = media.duration().unwrap_or(0.0);
    ui.label(tokens.muted(format!("{} / {}", clock(media.position), clock(duration))));
    let id = Id::NULL.with(("media-seek", pane));
    let dragging: Option<f64> = ui.data(|d| d.get_temp(id));
    let mut position = dragging.unwrap_or(media.position).clamp(0.0, duration);
    let response = ui.add_enabled(
        duration > 0.0,
        egui::Slider::new(&mut position, 0.0..=duration.max(0.1))
            .show_value(false)
            .text("Seek"),
    );
    if response.dragged() {
        ui.data_mut(|d| d.insert_temp(id, position));
    } else {
        ui.data_mut(|d| d.remove::<f64>(id));
        if response.changed() || response.drag_stopped() {
            actions.push(("media.seek", json!({"to": position})));
        }
    }
    response.has_focus()
}

fn volume_slider(ui: &mut Ui, s: &Session, actions: &mut Vec<(&'static str, Value)>) -> bool {
    let mut volume = s.media.volume;
    let response = ui.add(egui::Slider::new(&mut volume, 0.0..=100.0).text("Volume"));
    if response.changed() {
        actions.push(("media.volume", json!({"value": volume})));
    }
    response.has_focus()
}

/// Audio and subtitle pickers; they appear only when the video has a choice.
fn track_menus(ui: &mut Ui, s: &Session, actions: &mut Vec<(&'static str, Value)>) {
    let Some(video) = &s.media.video else {
        return;
    };
    let tracks = &video.info.tracks;
    if tracks.audio.len() > 1 {
        ui.menu_button("Audio track", |ui| {
            for (index, track) in tracks.audio.iter().enumerate() {
                let usable = track.format.is_some();
                let label = if usable {
                    track.label.clone()
                } else {
                    format!("{} (unsupported)", track.label)
                };
                let button = egui::Button::new(label).selected(index == tracks.selected_audio);
                if ui.add_enabled(usable, button).clicked() {
                    actions.push(("media.track.audio", json!({"index": index})));
                    ui.close();
                }
            }
        });
    }
    if !tracks.subtitles.is_empty() {
        ui.menu_button("Subtitles", |ui| {
            let current = s.media.subtitle_track;
            if ui
                .add(egui::Button::new("Off").selected(current.is_none()))
                .clicked()
            {
                actions.push(("media.track.subtitle", json!({"index": null})));
                ui.close();
            }
            for (index, track) in tracks.subtitles.iter().enumerate() {
                let button = egui::Button::new(&track.label).selected(current == Some(index));
                if ui.add(button).clicked() {
                    actions.push(("media.track.subtitle", json!({"index": index})));
                    ui.close();
                }
            }
        });
    }
}

fn keys_help(ui: &mut Ui) {
    egui::Grid::new("media-keys-help").show(ui, |ui| {
        for (keys, effect) in crate::media_keys::HELP {
            ui.monospace(*keys);
            ui.label(*effect);
            ui.end_row();
        }
    });
}

/// Fullscreen: only the video, with controls that fade after [`HIDE_AFTER`].
fn fullscreen(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let mut actions: Vec<(&str, Value)> = Vec::new();
    let now = ui.input(|i| i.time);
    let id = Id::NULL.with(("media-activity", pane));
    let active = ui.input(|i| {
        i.pointer.delta() != egui::Vec2::ZERO
            || i.pointer.any_down()
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { .. }))
    });
    let last: Option<f64> = ui.data(|d| d.get_temp(id));
    let last = match last {
        Some(last) if !active => last,
        _ => {
            ui.data_mut(|d| d.insert_temp(id, now));
            now
        }
    };
    let visible = controls_visible(now - last);
    let area = ui.available_size();
    video(ui, s, pane, area, tokens, &mut actions);
    let video_rect = ui.min_rect();
    let mut slider_focus = false;
    if visible {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(
                HIDE_AFTER - (now - last),
            ));
        egui::Area::new(Id::NULL.with(("media-fullscreen-controls", pane)))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -16.0))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.spacing_mut().slider_width = 140.0;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            for (label, command) in [
                                ("Previous", "media.prev"),
                                (
                                    if s.media.playing { "Pause" } else { "Play" },
                                    "media.toggle",
                                ),
                                ("Next", "media.next"),
                            ] {
                                if ui
                                    .add_enabled(
                                        s.command_enabled(command),
                                        egui::Button::new(label),
                                    )
                                    .clicked()
                                {
                                    actions.push((command, Value::Null));
                                }
                            }
                            slider_focus |= seek_slider(ui, s, pane, tokens, &mut actions);
                        });
                        ui.horizontal(|ui| {
                            slider_focus |= volume_slider(ui, s, &mut actions);
                            track_menus(ui, s, &mut actions);
                            ui.menu_button("Keys", keys_help);
                            if ui
                                .button("Exit fullscreen")
                                .on_hover_text("Leave fullscreen (Esc or F)")
                                .clicked()
                            {
                                actions.push(("media.fullscreen", json!({"on": false})));
                            }
                        });
                    });
                });
            });
    } else if ui.rect_contains_pointer(video_rect) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    }
    crate::media_keys::note_slider_focus(ui.ctx(), slider_focus);
    for (command, params) in actions {
        s.dispatch_or_notice(command, &params);
    }
}

fn video(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    size: egui::Vec2,
    tokens: &Tokens,
    actions: &mut Vec<(&'static str, Value)>,
) {
    let width = size.x.max(1.0);
    let pixels = width * ui.ctx().pixels_per_point();
    let bucket = fm_engine::video_cmds::width_bucket(pixels);
    let state = s.media.video.as_ref();
    if state.and_then(|video| video.width) != Some(bucket) {
        actions.push(("media.video.fit", json!({"width": pixels})));
    }
    let height = size.y.max(1.0);
    // Not focusable: clicking the picture must not steal the arrow keys from the player.
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::CLICK);
    ui.painter().rect_filled(rect, 0.0, tokens.bg);
    if let Some(frame) = state.and_then(|video| video.frame.as_ref()) {
        let texture =
            crate::preview::video_texture(ui.ctx(), Id::NULL.with(("media-video", pane)), frame);
        let scale = (width / frame.width.max(1) as f32).min(height / frame.height.max(1) as f32);
        let size = egui::vec2(frame.width as f32, frame.height as f32) * scale;
        egui::Image::from_texture(&texture)
            .paint_at(ui, egui::Rect::from_center_size(rect.center(), size));
    } else if s.media.playing {
        ui.put(rect, egui::Label::new("Preparing video..."));
    }
    let click_id = Id::NULL.with(("media-click", pane));
    let now = ui.input(|i| i.time);
    if response.double_clicked() {
        ui.data_mut(|d| d.remove::<f64>(click_id));
        actions.push(("media.fullscreen", Value::Null));
    } else if response.clicked() {
        ui.data_mut(|d| d.insert_temp(click_id, now));
    }
    if let Some(at) = ui.data(|d| d.get_temp::<f64>(click_id)) {
        if now - at >= CLICK_DELAY {
            ui.data_mut(|d| d.remove::<f64>(click_id));
            actions.push(("media.toggle", Value::Null));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(CLICK_DELAY));
        }
    }
    if s.media.playing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

pub fn indicator(ui: &mut Ui, s: &mut Session, tokens: &Tokens) {
    if !s.media.playing {
        return;
    }
    let title = s
        .media
        .info
        .as_ref()
        .and_then(|info| info.tags.title.as_deref())
        .or_else(|| s.media.queue.current().and_then(|uri| uri.file_name()))
        .unwrap_or("Audio");
    ui.add_sized(
        [180.0, ui.spacing().interact_size.y],
        egui::Label::new(tokens.muted(format!("Playing: {title}"))).truncate(),
    );
    let stop = if s.media.is_video() {
        "Stop video"
    } else {
        "Stop audio"
    };
    if ui.small_button(stop).clicked() {
        s.dispatch_or_notice("media.stop", &Value::Null);
    }
}

fn cover(ui: &mut Ui, pane: PaneId, image: Option<&Arc<fm_preview::ImageDoc>>, tokens: &Tokens) {
    let id = Id::NULL.with(("media-cover", pane));
    let Some(image) = image else {
        ui.data_mut(|d| d.remove::<(Arc<fm_preview::ImageDoc>, egui::TextureHandle)>(id));
        ui.allocate_ui(egui::vec2(96.0, 96.0), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(tokens.muted("Audio"));
            });
        });
        return;
    };
    let Some((rgba, _)) = image.frames.first() else {
        return;
    };
    let cached = ui.data(|d| d.get_temp::<(Arc<fm_preview::ImageDoc>, egui::TextureHandle)>(id));
    let texture =
        if let Some((_, texture)) = cached.filter(|(previous, _)| Arc::ptr_eq(previous, image)) {
            texture
        } else {
            let texture = ui.ctx().load_texture(
                "media-cover",
                egui::ColorImage::from_rgba_unmultiplied(
                    [image.width as usize, image.height as usize],
                    rgba,
                ),
                egui::TextureOptions::LINEAR,
            );
            ui.data_mut(|d| d.insert_temp(id, (image.clone(), texture.clone())));
            texture
        };
    let scale = 96.0 / image.width.max(image.height).max(1) as f32;
    ui.add(
        egui::Image::from_texture(&texture).fit_to_exact_size(egui::vec2(
            image.width as f32 * scale,
            image.height as f32 * scale,
        )),
    );
}

fn spool_label(done: u64, total: u64) -> String {
    let mib = |bytes: u64| bytes as f64 / (1024.0 * 1024.0);
    if total > 0 {
        format!("Downloading {:.1} / {:.1} MiB", mib(done), mib(total))
    } else {
        format!("Downloading {:.1} MiB", mib(done))
    }
}

pub(crate) fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn harness(tokens: Tokens) -> Harness<'static, Session> {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/music/one.mp3", "/music/two.mp3"]}),
        )
        .expect("enqueue");
        let pane = s.media_pane().expect("media");
        Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    egui::CentralPanel::default().show(ui, |ui| {
                        show(ui, s, pane, &tokens);
                    });
                },
                s,
            )
    }

    #[test]
    fn audio_controls_dispatch_in_both_themes_without_starting_playback() {
        for tokens in [Tokens::dark(), Tokens::light()] {
            let mut h = harness(tokens);
            h.get_by_label("Mute").click();
            h.run();
            assert!(h.state().media.muted);
            h.get_by_label("Shuffle").click();
            h.run();
            assert!(h.state().media.queue.shuffle());
            h.get_by_label("Keep audio playing when the Media pane is closed")
                .click();
            h.run();
            assert!(!h.state().prefs.media_background_audio);
            assert!(!h.state().media.playing);
            h.get_by_label("Clear queue").click();
            h.run();
            assert!(h.state().media.queue.items().is_empty());
        }
    }

    #[test]
    fn background_indicator_can_stop_playback() {
        let mut s = Session::new().expect("session");
        s.media
            .queue
            .enqueue(fm_vfs::VfsUri::parse("/music/one.mp3").expect("uri"))
            .expect("enqueue");
        s.media.queue.select(0).expect("current");
        s.media.playing = true;
        s.audio_owner = fm_engine::audio_owner::AudioOwner::Media;
        let mut h = Harness::builder().build_ui_state(
            |ui, s| {
                indicator(ui, s, &Tokens::dark());
            },
            s,
        );
        h.get_by_label("Stop audio").click();
        h.run();
        assert!(!h.state().media.playing);
        assert_eq!(
            h.state().audio_owner,
            fm_engine::audio_owner::AudioOwner::None
        );
    }
}
