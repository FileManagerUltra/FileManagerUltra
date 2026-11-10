//! Media is a view of session playback; all controls dispatch engine commands.

use egui::{Id, ScrollArea, Ui};
use fm_engine::Session;
use fm_layout::PaneId;
use fm_media::RepeatMode;
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
    if s.media.is_video() {
        let max = video_max(ui);
        let height = video_height(ui, pane);
        video(
            ui,
            s,
            pane,
            egui::vec2(ui.available_width(), height),
            tokens,
            &mut actions,
        );
        video_splitter(ui, pane, height, max, tokens);
    }
    let top = ui.cursor().top();
    display(ui, s, pane, tokens, &mut actions);
    transport(ui, s, tokens, &mut actions);
    crate::media_book::controls(ui, s, tokens, &mut actions);
    if s.media.is_video() {
        let used = ui.cursor().top() - top;
        ui.data_mut(|d| d.insert_temp(Id::NULL.with("media-controls-height"), used));
    }
    if let Some(hint) = s.media.video.as_ref().and_then(|video| video.hint.as_ref()) {
        crate::dialog_style::hint(ui, tokens, hint);
    }
    if let Some(error) = &s.media.error {
        ui.colored_label(tokens.error, error);
    }
    if let Some((done, total)) = s.media.spool_progress() {
        let bar = if total > 0 {
            egui::ProgressBar::new((done as f64 / total as f64) as f32)
        } else {
            egui::ProgressBar::new(0.0).animate(true)
        };
        ui.add(bar.text(spool_label(done, total)));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    playlist(ui, s, pane, tokens, &mut actions);
    // The painted bars are not keyboard-focusable, so the media keys are never shadowed.
    crate::media_keys::note_slider_focus(ui.ctx(), false);
    if !actions.is_empty() {
        ui.ctx().request_repaint();
    }
    for (command, params) in actions {
        s.dispatch_or_notice(command, &params);
    }
}

const MIN_VIDEO_HEIGHT: f32 = 64.0;
/// Room kept below the video so the display and controls never leave the pane: their height
/// from the previous frame (a first-frame estimate), the handle and a little playlist.
fn reserved_below(ui: &Ui) -> f32 {
    let controls: f32 = ui
        .data(|d| d.get_temp(Id::NULL.with("media-controls-height")))
        .unwrap_or(140.0);
    controls + 6.0 + 40.0
}

fn video_max(ui: &Ui) -> f32 {
    (ui.available_height() - reserved_below(ui)).max(MIN_VIDEO_HEIGHT)
}

/// The user's chosen video height, or half the pane by default, kept within the pane.
fn video_height(ui: &Ui, pane: PaneId) -> f32 {
    let id = Id::NULL.with(("media-video-height", pane));
    let chosen: Option<f32> = ui.data(|d| d.get_temp(id));
    chosen
        .unwrap_or(ui.available_height() * 0.5)
        .clamp(MIN_VIDEO_HEIGHT, video_max(ui))
}

/// Drag handle under the video; the picture refits to the new height.
fn video_splitter(ui: &mut Ui, pane: PaneId, height: f32, max: f32, tokens: &Tokens) {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 6.0),
        egui::Sense::click_and_drag(),
    );
    let response = response
        .on_hover_cursor(egui::CursorIcon::ResizeVertical)
        .on_hover_text("Drag to resize the video");
    let id = Id::NULL.with(("media-video-height", pane));
    if response.dragged() {
        let height = (height + response.drag_delta().y).clamp(MIN_VIDEO_HEIGHT, max);
        ui.data_mut(|d| d.insert_temp(id, height));
    }
    let color = if response.hovered() || response.dragged() {
        tokens.accent_text
    } else {
        tokens.border
    };
    ui.painter().line_segment(
        [
            rect.center() - egui::vec2(16.0, 0.0),
            rect.center() + egui::vec2(16.0, 0.0),
        ],
        egui::Stroke::new(2.0, color),
    );
}

fn title_of(media: &fm_engine::media_cmds::MediaSession) -> String {
    media
        .info
        .as_ref()
        .and_then(|info| info.tags.title.clone())
        .or_else(|| {
            media
                .queue
                .current()
                .and_then(|uri| uri.file_name())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "No media selected".into())
}

/// The "now playing" panel: artwork, title, state chips, big clock and the seek bar.
fn display(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<(&'static str, Value)>,
) {
    let media = &s.media;
    egui::Frame::new()
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(tokens.border_width, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if !media.is_video() {
                    cover(ui, pane, media.cover.as_ref(), tokens);
                }
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(title_of(media)).strong().size(15.0))
                            .truncate(),
                    );
                    if let Some(info) = &media.info {
                        let line = [&info.tags.artist, &info.tags.album]
                            .into_iter()
                            .flatten()
                            .map(String::as_str)
                            .collect::<Vec<_>>()
                            .join(" – ");
                        if !line.is_empty() {
                            ui.add(egui::Label::new(tokens.muted(line)).truncate());
                        }
                    }
                    // A book shows its whole-book time so the clock agrees with the seek bar.
                    let book = media
                        .book_view()
                        .and_then(|v| v.total.map(|t| (v.position, t)));
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(clock(book.map_or(media.position, |b| b.0)))
                                .monospace()
                                .size(22.0)
                                .color(tokens.accent_text),
                        );
                        ui.label(tokens.muted(format!(
                            "/ {}",
                            book.map(|b| b.1)
                                .or_else(|| media.duration())
                                .map_or("--:--".into(), clock)
                        )));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            for (text, tip) in chips(s).into_iter().rev() {
                                egui::Frame::new()
                                    .fill(tokens.selection_bg)
                                    .corner_radius(8.0)
                                    .inner_margin(egui::Margin::symmetric(7, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            egui::RichText::new(text)
                                                .small()
                                                .strong()
                                                .color(tokens.accent_text),
                                        );
                                    })
                                    .response
                                    .on_hover_text(tip);
                            }
                        });
                    });
                });
            });
            let width = ui.available_width();
            seek_bar(ui, s, pane, tokens, width, actions);
        });
}

/// Status badges (text, tooltip) for the features that live in the menu.
fn chips(s: &Session) -> Vec<(String, String)> {
    let extras = &s.media.extras;
    let mut chips = Vec::new();
    if s.media.gain_eq().enabled {
        chips.push(("EQ".to_string(), "Equalizer is on".to_string()));
    }
    if let Some(a) = extras.loop_a {
        chips.push((
            format!(
                "A-B {}–{}",
                clock(a),
                extras.loop_b.map_or("…".into(), clock)
            ),
            "Looping between two points".into(),
        ));
    }
    if let Some(left) = extras.sleep_remaining() {
        chips.push((
            format!("Sleep {}m", left.as_secs().div_ceil(60)),
            "Sleep timer: playback stops when it runs out".into(),
        ));
    } else if extras.sleep.is_some() {
        chips.push((
            "Sleep: end of item".into(),
            "Playback stops at the end of this item".into(),
        ));
    }
    if let Some(view) = s.media.book_view() {
        if view.finished {
            chips.push(("Finished".into(), "Marked as finished".into()));
        } else if let Some(total) = view.total.filter(|t| *t > 0.0) {
            let percent = (view.position / total * 100.0).floor().clamp(0.0, 100.0);
            chips.push((format!("{percent}%"), "Progress through the book".into()));
        }
    }
    if s.media.speed != 1.0 {
        chips.push((format!("{}×", s.media.speed), "Playback speed".into()));
    }
    chips
}

/// Icon row: transport on the left, modes and volume after it, the options menu last.
fn transport(ui: &mut Ui, s: &Session, tokens: &Tokens, actions: &mut Vec<(&'static str, Value)>) {
    use crate::media_icons::{Icon, button};
    let media = &s.media;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let on = |command| s.command_enabled(command);
        if button(ui, tokens, Icon::Prev, "Previous", on("media.prev"), false).clicked() {
            actions.push(("media.prev", Value::Null));
        }
        let (icon, label) = if media.playing {
            (Icon::Pause, "Pause")
        } else {
            (Icon::Play, "Play")
        };
        if button(ui, tokens, icon, label, on("media.toggle"), false).clicked() {
            actions.push(("media.toggle", Value::Null));
        }
        if button(ui, tokens, Icon::Stop, "Stop", on("media.stop"), false).clicked() {
            actions.push(("media.stop", Value::Null));
        }
        if button(ui, tokens, Icon::Next, "Next", on("media.next"), false).clicked() {
            actions.push(("media.next", Value::Null));
        }
        if media.is_video() {
            ui.separator();
            for (icon, label, frames) in [
                (Icon::FrameBack, "Previous frame", -1),
                (Icon::FrameForward, "Next frame", 1),
            ] {
                if button(ui, tokens, icon, label, on("media.step"), false).clicked() {
                    actions.push(("media.step", json!({"frames": frames})));
                }
            }
            if button(
                ui,
                tokens,
                Icon::Fullscreen,
                "Fullscreen (F)",
                on("media.fullscreen"),
                false,
            )
            .clicked()
            {
                actions.push(("media.fullscreen", Value::Null));
            }
        }
        ui.separator();
        let shuffle = media.queue.shuffle();
        if button(ui, tokens, Icon::Shuffle, "Shuffle", true, shuffle).clicked() {
            actions.push(("media.shuffle", json!({"on": !shuffle})));
        }
        let (icon, label, next, active) = match media.queue.repeat() {
            RepeatMode::Off => (Icon::Repeat, "Repeat: off", "all", false),
            RepeatMode::All => (Icon::Repeat, "Repeat: all", "one", true),
            RepeatMode::One => (Icon::RepeatOne, "Repeat: one", "off", true),
        };
        if button(ui, tokens, icon, label, true, active).clicked() {
            actions.push(("media.repeat", json!({"mode": next})));
        }
        ui.separator();
        volume(ui, s, tokens, actions);
        ui.separator();
        let menu = button(ui, tokens, Icon::Menu, "Options", true, false);
        egui::Popup::menu(&menu).show(|ui| options_menu(ui, s, actions));
        crate::media_audio::equalizer_window(ui, s, actions);
    });
}

/// Everything that is not needed during normal listening.
fn options_menu(ui: &mut Ui, s: &Session, actions: &mut Vec<(&'static str, Value)>) {
    crate::media_audio::menus(ui, s, actions);
    if s.media.video.is_some() {
        ui.menu_button("Speed", |ui| {
            for speed in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
                let selected = (s.media.speed - speed).abs() < 0.01;
                if ui
                    .add(egui::Button::selectable(selected, format!("{speed}×")))
                    .clicked()
                {
                    actions.push(("media.speed", json!({"value": speed})));
                    ui.close();
                }
            }
        });
    }
    track_menus(ui, s, actions);
    ui.separator();
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
        .checkbox(&mut background, "Keep playing when the pane is closed")
        .changed()
    {
        actions.push((
            "prefs.set",
            json!({"key": "media_background_audio", "value": background}),
        ));
    }
    ui.separator();
    ui.menu_button("Keyboard shortcuts", keys_help);
}

/// Width left of the number column for the folder arrow, padlock and play marker.
const GUTTER: f32 = 26.0;

/// Playlist: numbered rows, the current one highlighted with a play marker.
fn playlist(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    tokens: &Tokens,
    actions: &mut Vec<(&'static str, Value)>,
) {
    let media = &s.media;
    let count = media.queue.items().len();
    let current = media.queue.current_index();
    ui.add_space(2.0);
    let rename_id = Id::NULL.with(("media-rename", pane));
    let fresh_id = Id::NULL.with(("media-new-folder", pane));
    let mut renaming: Option<(u32, String, bool)> = ui.data(|d| d.get_temp(rename_id)).flatten();
    ui.horizontal(|ui| {
        ui.label(tokens.muted(match count {
            0 => "Playlist".to_string(),
            1 => "Playlist · 1 item".to_string(),
            n => format!("Playlist · {n} items"),
        }));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::media_icons::button(
                ui,
                tokens,
                crate::media_icons::Icon::Clear,
                "Clear playlist",
                count > 0,
                false,
            )
            .clicked()
            {
                actions.push(("media.clear", Value::Null));
            }
            if crate::media_icons::button(
                ui,
                tokens,
                crate::media_icons::Icon::NewFolder,
                "New folder",
                true,
                false,
            )
            .clicked()
            {
                let index = s.layout.state(pane).and_then(|state| state.media.selected);
                actions.push((
                    "media.group_create",
                    index.map_or(Value::Null, |index| json!({"index": index})),
                ));
                ui.data_mut(|d| d.insert_temp(fresh_id, true));
            }
        });
    });
    if ui.data(|d| d.get_temp::<bool>(fresh_id)).unwrap_or(false) && actions.is_empty() {
        // The folder was created by last frame's command: start naming it.
        ui.data_mut(|d| d.remove::<bool>(fresh_id));
        if let Some(group) = media.queue.groups().into_iter().max_by_key(|g| g.id) {
            renaming = Some((group.id, group.name, false));
        }
    }
    if count == 0 {
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
    let multi_id = Id::NULL.with(("media-multi", pane));
    let (mut multi, multi_len): (Vec<usize>, usize) =
        ui.data(|d| d.get_temp(multi_id)).unwrap_or_default();
    if multi_len != count {
        multi.clear();
    }
    let digits = count.to_string().len().max(2);
    let row_height = tokens.row_height;
    let found = media.queue.groups();
    let collapsed_id = Id::NULL.with(("media-collapsed", pane));
    let mut collapsed: Vec<u32> = ui.data(|d| d.get_temp(collapsed_id)).unwrap_or_default();
    let rows = playlist_rows(&found, &collapsed, count);
    // A folder counts as one entry in the playlist numbering; its files are numbered inside it.
    let mut entry = 0;
    let numbers: Vec<usize> = rows
        .iter()
        .map(|row| {
            if !matches!(row, PlaylistRow::Item { grouped: true, .. }) {
                entry += 1;
            }
            entry
        })
        .collect();
    let mut toggle = None;
    let output = ScrollArea::vertical()
        .id_salt(("media-queue", pane))
        .vertical_scroll_offset(scroll)
        .auto_shrink(false)
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            for row_at in range {
                let (index, grouped) = match rows.get(row_at) {
                    Some(&PlaylistRow::Item { index, grouped }) => (index, grouped),
                    Some(&PlaylistRow::Header(group)) => {
                        if let Some(group) = found.get(group) {
                            let can_continue = group.dir.as_deref().is_some_and(|dir| {
                                media
                                    .queue
                                    .items()
                                    .get(group.start..group.start + group.len)
                                    .is_some_and(|members| media.resume_in(dir, members).is_some())
                            });
                            let view = HeaderView {
                                collapsed: collapsed.contains(&group.id),
                                number: numbers.get(row_at).copied().unwrap_or(0),
                                digits,
                                can_continue,
                                current,
                                book: media.book_menu(group.start),
                            };
                            group_header(
                                ui,
                                group,
                                view,
                                (tokens, &mut *actions),
                                (&mut toggle, &mut renaming),
                            );
                        }
                        continue;
                    }
                    None => continue,
                };
                let Some(uri) = media.queue.items().get(index) else {
                    continue;
                };
                let name = uri.file_name().unwrap_or("Media");
                let name = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
                let is_current = Some(index) == current;
                let (rect, row) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), row_height),
                    egui::Sense::click_and_drag(),
                );
                row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
                if row.drag_started() && !media.queue.is_locked(index) {
                    egui::DragAndDrop::set_payload(ui.ctx(), PlaylistDrag::Row(index));
                }
                if row.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                let fraction = ui
                    .ctx()
                    .pointer_latest_pos()
                    .map_or(0.5, |pos| (pos.y - rect.top()) / rect.height());
                let last_in_folder = media.queue.group_of(index).is_some_and(|g| g.last);
                let locked = media.queue.is_locked(index);
                let zone = if index == 0 && !grouped && fraction < 1.0 / 3.0 {
                    DropZone::Before
                } else if last_in_folder && fraction > 2.0 / 3.0 {
                    DropZone::AfterFolder
                } else if locked {
                    DropZone::Refused
                } else {
                    DropZone::After
                };
                if let Some(from) = crate::dnd::release_payload::<PlaylistDrag>(&row) {
                    drop_on_row(*from, index, zone, actions);
                }
                // Files dropped on a row of a folder join that folder.
                let files_here = row.contains_pointer()
                    && egui::DragAndDrop::has_payload_of_type::<crate::dnd::DragPayload>(ui.ctx());
                if files_here {
                    let slot = match zone {
                        DropZone::Before => json!({"before": index}),
                        DropZone::After => json!({"after": index}),
                        DropZone::AfterFolder => json!({"after": index, "outside": true}),
                        DropZone::Refused => Value::Null,
                    };
                    ui.data_mut(|d| d.insert_temp(drop_slot_id(), slot));
                }
                if (row.dnd_hover_payload::<PlaylistDrag>().is_some() || files_here)
                    && zone != DropZone::Refused
                {
                    let (y, from) = match zone {
                        DropZone::Refused => (rect.top(), rect.left()),
                        DropZone::Before => (rect.top(), rect.left()),
                        DropZone::After => (rect.bottom(), rect.left() + 20.0),
                        DropZone::AfterFolder => (rect.bottom(), rect.left()),
                    };
                    ui.ctx()
                        .layer_painter(egui::LayerId::new(
                            egui::Order::Foreground,
                            Id::NULL.with("playlist_drop_line"),
                        ))
                        .with_clip_rect(ui.clip_rect())
                        .hline(
                            from..=rect.right(),
                            y,
                            egui::Stroke::new(2.0, tokens.accent_text),
                        );
                }
                let painter = ui.painter();
                if selection == Some(index) || multi.contains(&index) {
                    painter.rect_filled(rect, 2.0, tokens.selection_bg);
                } else if row.hovered() {
                    painter.rect_filled(rect, 2.0, tokens.hover_bg);
                }
                let font = egui::TextStyle::Body.resolve(ui.style());
                let color = if is_current {
                    tokens.accent_text
                } else {
                    tokens.text
                };
                let mid = rect.center().y;
                if is_current {
                    let x = rect.left() + 8.0 + if grouped { 18.0 } else { 0.0 };
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(x - 3.0, mid - 4.5),
                            egui::pos2(x - 3.0, mid + 4.5),
                            egui::pos2(x + 4.0, mid),
                        ],
                        color,
                        egui::Stroke::NONE,
                    ));
                }
                if !is_current && let Some(mark) = s.media.row_mark(index) {
                    let x = rect.left() + 8.0 + if grouped { 18.0 } else { 0.0 } + 11.0;
                    let tip = match mark {
                        fm_engine::media_book::RowMark::Resume(at) => {
                            format!("Resumes at {}", clock(at))
                        }
                        fm_engine::media_book::RowMark::Played => "Played".to_string(),
                        fm_engine::media_book::RowMark::Finished => "Finished".to_string(),
                    };
                    crate::media_icons::paint_row_mark(
                        painter,
                        egui::pos2(x, mid),
                        mark,
                        tokens.accent_text,
                    );
                    ui.interact(
                        egui::Rect::from_center_size(
                            egui::pos2(x, mid),
                            egui::vec2(12.0, rect.height()),
                        ),
                        Id::NULL.with(("media-row-mark", index)),
                        egui::Sense::hover(),
                    )
                    .on_hover_text(tip);
                }
                // Folder rows are indented under their header and numbered within the folder.
                let (indent, number) = match found
                    .iter()
                    .find(|g| index >= g.start && index < g.start + g.len)
                {
                    Some(g) if grouped => (18.0, index - g.start + 1),
                    _ => (0.0, numbers.get(row_at).copied().unwrap_or(index + 1)),
                };
                if grouped {
                    // A tree guide from under the folder arrow, ending in a tick at the last file.
                    let gap = ui.spacing().item_spacing.y / 2.0;
                    let stroke = egui::Stroke::new(1.0, tokens.text_muted);
                    let x = rect.left() + 10.0;
                    let bottom = if last_in_folder {
                        mid
                    } else {
                        rect.bottom() + gap
                    };
                    painter.vline(x, (rect.top() - gap)..=bottom, stroke);
                    if last_in_folder {
                        painter.hline(x..=x + 9.0, mid, stroke);
                    }
                }
                painter.text(
                    egui::pos2(
                        rect.left() + 16.0 + GUTTER + indent + digits as f32 * 8.0,
                        mid,
                    ),
                    egui::Align2::RIGHT_CENTER,
                    format!("{number}."),
                    font.clone(),
                    tokens.text_muted,
                );
                let text_x = rect.left() + 24.0 + GUTTER + indent + digits as f32 * 8.0;
                painter.with_clip_rect(rect.with_min_x(text_x)).text(
                    egui::pos2(text_x, mid),
                    egui::Align2::LEFT_CENTER,
                    name,
                    font,
                    color,
                );
                if row.clicked() {
                    let mods = ui.input(|i| i.modifiers);
                    if mods.shift
                        && let Some(anchor) = selection
                    {
                        let (lo, hi) = (anchor.min(index), anchor.max(index));
                        multi = (lo..=hi).collect();
                    } else if mods.command {
                        if multi.is_empty() {
                            multi.extend(selection);
                        }
                        match multi.iter().position(|m| *m == index) {
                            Some(at) => {
                                multi.remove(at);
                            }
                            None => multi.push(index),
                        }
                        selection = Some(index);
                    } else {
                        multi.clear();
                        selection = Some(index);
                    }
                }
                if row.double_clicked() {
                    actions.push(("media.play", json!({"index": index})));
                }
                row.context_menu(|ui| {
                    if ui.button("Play").clicked() {
                        actions.push(("media.play", json!({"index": index})));
                        ui.close();
                    }
                    let book = media.book_menu(index);
                    if book.is_book
                        && ui
                            .button("Play from start")
                            .on_hover_text("Ignore the remembered position")
                            .clicked()
                    {
                        actions.push(("media.play", json!({"index": index, "from_start": true})));
                        ui.close();
                    }
                    crate::media_book::menu_items(ui, book, index, actions);
                    let many = multi.len() > 1 && multi.contains(&index);
                    let label = if many {
                        format!("Remove {} items", multi.len())
                    } else {
                        "Remove".to_string()
                    };
                    if ui.button(label).clicked() {
                        let rows = if many { multi.clone() } else { vec![index] };
                        remove_rows(&rows, actions);
                        multi.clear();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("New folder from this").clicked() {
                        actions.push(("media.group_create", json!({"index": index})));
                        ui.data_mut(|d| d.insert_temp(fresh_id, true));
                        ui.close();
                    }
                    if !found.is_empty() {
                        ui.menu_button("Move to folder", |ui| {
                            for group in &found {
                                if ui.button(&group.name).clicked() {
                                    actions.push((
                                        "media.group_add",
                                        json!({"index": index, "group": group.id}),
                                    ));
                                    ui.close();
                                }
                            }
                        });
                    }
                    if ui
                        .add_enabled(grouped, egui::Button::new("Remove from folder"))
                        .clicked()
                    {
                        actions.push(("media.ungroup", json!({"index": index})));
                        ui.close();
                    }
                    ui.separator();
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
                        .add_enabled(index + 1 < count, egui::Button::new("Move down"))
                        .clicked()
                    {
                        actions.push(("media.move", json!({"from": index, "to": index + 1})));
                        ui.close();
                    }
                });
            }
        });
    if let Some(id) = toggle {
        match collapsed.iter().position(|d| *d == id) {
            Some(at) => {
                collapsed.remove(at);
            }
            None => collapsed.push(id),
        }
        ui.data_mut(|d| d.insert_temp(collapsed_id, collapsed));
    }
    ui.data_mut(|d| d.insert_temp(rename_id, renaming));
    // The app's Delete shortcut leaves a request when this pane has the focus.
    let delete = s.layout.focused() == pane
        && ui.data_mut(|d| d.remove_temp::<bool>(delete_request_id()).unwrap_or(false));
    if delete {
        let rows: Vec<usize> = if multi.is_empty() {
            selection.into_iter().collect()
        } else {
            multi.clone()
        };
        remove_rows(&rows, actions);
        multi.clear();
        selection = None;
    }
    ui.data_mut(|d| d.insert_temp(multi_id, (multi, count)));
    if let Some(state) = s.layout.state_mut(pane) {
        state.media.selected = selection;
        state.media.queue_scroll = output.state.clamped_offset().y;
    }
}

/// Set by the Delete shortcut while a Media pane is focused.
pub fn delete_request_id() -> Id {
    Id::NULL.with("media-delete-request")
}

/// Remove several rows; highest first so the remaining indices stay valid.
fn remove_rows(rows: &[usize], actions: &mut Vec<(&'static str, Value)>) {
    let mut rows = rows.to_vec();
    rows.sort_unstable_by(|a, b| b.cmp(a));
    rows.dedup();
    for index in rows {
        actions.push(("media.remove", json!({"index": index})));
    }
}

/// What a playlist drag carries.
#[derive(Clone, Copy, Debug)]
enum PlaylistDrag {
    Row(usize),
    Folder(u32),
}

enum PlaylistRow {
    Header(usize),
    Item { index: usize, grouped: bool },
}

/// Where a playlist drag lands relative to the row or folder header it is released on.
#[derive(Clone, Copy, PartialEq)]
enum DropZone {
    /// Just before the row (or before the whole folder, on a header).
    Before,
    /// Just after the row, inside its folder; on a header, first inside the folder.
    After,
    /// Just after the whole folder.
    AfterFolder,
    /// A locked folder takes no drops inside.
    Refused,
}

/// Dropped on an item row: right after it (inside its folder), with edge zones for the very
/// first row and for stepping out behind a folder.
fn drop_on_row(
    from: PlaylistDrag,
    onto: usize,
    zone: DropZone,
    actions: &mut Vec<(&'static str, Value)>,
) {
    actions.push(match (from, zone) {
        (_, DropZone::Refused) => return,
        (PlaylistDrag::Row(index), DropZone::Before) => {
            ("media.place", json!({"index": index, "before": onto}))
        }
        (PlaylistDrag::Row(index), DropZone::After) => {
            ("media.place", json!({"index": index, "after": onto}))
        }
        (PlaylistDrag::Row(index), DropZone::AfterFolder) => (
            "media.place",
            json!({"index": index, "after": onto, "outside": true}),
        ),
        (PlaylistDrag::Folder(id), DropZone::Before) => {
            ("media.group_place", json!({"id": id, "before": onto}))
        }
        (PlaylistDrag::Folder(id), _) => ("media.group_place", json!({"id": id, "after": onto})),
    });
}

/// Display rows: a header before each folder, whose files hide while collapsed.
fn playlist_rows(groups: &[fm_media::Group], collapsed: &[u32], count: usize) -> Vec<PlaylistRow> {
    let mut rows = Vec::with_capacity(count + groups.len());
    let mut next = groups
        .iter()
        .enumerate()
        .filter(|(_, g)| g.len > 0)
        .peekable();
    let mut index = 0;
    while index < count {
        match next.peek() {
            Some(&(at, group)) if group.start == index => {
                rows.push(PlaylistRow::Header(at));
                if !collapsed.contains(&group.id) {
                    rows.extend((index..index + group.len).map(|index| PlaylistRow::Item {
                        index,
                        grouped: true,
                    }));
                }
                index += group.len;
                next.next();
            }
            _ => {
                rows.push(PlaylistRow::Item {
                    index,
                    grouped: false,
                });
                index += 1;
            }
        }
    }
    rows.extend(
        groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.len == 0)
            .map(|(at, _)| PlaylistRow::Header(at)),
    );
    rows
}

#[derive(Clone, Copy)]
struct HeaderView {
    number: usize,
    digits: usize,
    collapsed: bool,
    can_continue: bool,
    current: Option<usize>,
    book: fm_engine::media_book::BookMenu,
}

/// Where a Files-pane drag lands in the queue: `media.drop` placement params, or null when
/// the spot refuses drops (read by `dnd::finish_media`).
pub fn drop_slot_id() -> Id {
    Id::NULL.with("media-drop-slot")
}

/// Where a Files-pane drag is released over an empty folder header (read by `dnd::finish_media`).
pub fn drop_group_id() -> Id {
    Id::NULL.with("media-drop-group")
}

fn group_header(
    ui: &mut Ui,
    group: &fm_media::Group,
    view: HeaderView,
    (tokens, actions): (&Tokens, &mut Vec<(&'static str, Value)>),
    (toggle, renaming): (&mut Option<u32>, &mut Option<(u32, String, bool)>),
) {
    let HeaderView {
        number,
        digits,
        collapsed,
        can_continue,
        current,
        book,
    } = view;
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), tokens.row_height),
        egui::Sense::hover(),
    );
    // A stable id keeps the double-click tracked while the first click collapses the group.
    let row = ui.interact(
        rect,
        Id::NULL.with(("media-group", group.id)),
        egui::Sense::click_and_drag(),
    );
    let title = if group.len == 0 {
        format!("{} · empty, drop files here", group.name)
    } else {
        format!("{} · {} files", group.name, group.len)
    };
    row.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &title));
    let fraction = ui
        .ctx()
        .pointer_latest_pos()
        .map_or(0.5, |pos| (pos.y - rect.top()) / rect.height());
    // Above the name: before the folder; on it: into the folder; a collapsed folder also has a
    // zone below the name for landing after it.
    let zone = if collapsed && fraction > 2.0 / 3.0 {
        DropZone::AfterFolder
    } else if fraction < if collapsed { 1.0 / 3.0 } else { 0.5 } {
        DropZone::Before
    } else if group.locked {
        DropZone::Refused
    } else {
        DropZone::After
    };
    let end = (group.start + group.len).saturating_sub(1);
    if let Some(from) = crate::dnd::release_payload::<PlaylistDrag>(&row) {
        match *from {
            PlaylistDrag::Row(index) if group.len > 0 => {
                actions.push(match zone {
                    DropZone::Before => (
                        "media.place",
                        json!({"index": index, "before": group.start, "outside": true}),
                    ),
                    DropZone::After => (
                        "media.place",
                        json!({"index": index, "before": group.start}),
                    ),
                    DropZone::AfterFolder => (
                        "media.place",
                        json!({"index": index, "after": end, "outside": true}),
                    ),
                    DropZone::Refused => return,
                });
            }
            PlaylistDrag::Row(index) => {
                actions.push((
                    "media.group_add",
                    json!({"index": index, "group": group.id}),
                ));
            }
            PlaylistDrag::Folder(id) if id != group.id && group.len > 0 => {
                actions.push(if zone == DropZone::Before {
                    (
                        "media.group_place",
                        json!({"id": id, "before": group.start}),
                    )
                } else {
                    ("media.group_place", json!({"id": id, "after": end}))
                });
            }
            PlaylistDrag::Folder(_) => {}
        }
    }
    if row.drag_started() && group.len > 0 {
        egui::DragAndDrop::set_payload(ui.ctx(), PlaylistDrag::Folder(group.id));
    }
    if row.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    let files_hover = row.contains_pointer()
        && egui::DragAndDrop::has_payload_of_type::<crate::dnd::DragPayload>(ui.ctx());
    let hovered_drop = ((row.dnd_hover_payload::<PlaylistDrag>().is_some() || files_hover)
        && zone != DropZone::Refused)
        || (files_hover && group.len == 0 && !group.locked);
    if files_hover {
        if group.len == 0 {
            if !group.locked {
                ui.data_mut(|d| d.insert_temp(drop_group_id(), group.id));
            }
        } else {
            let slot = match zone {
                DropZone::Before => json!({"before": group.start, "outside": true}),
                DropZone::After => json!({"before": group.start}),
                DropZone::AfterFolder => json!({"after": end, "outside": true}),
                DropZone::Refused => Value::Null,
            };
            ui.data_mut(|d| d.insert_temp(drop_slot_id(), slot));
        }
    }
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        2.0,
        if row.hovered() || hovered_drop {
            tokens.hover_bg
        } else {
            tokens.panel
        },
    );
    let line = egui::Stroke::new(2.0, tokens.accent_text);
    let fg = ui
        .ctx()
        .layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            Id::NULL.with("playlist_drop_line"),
        ))
        .with_clip_rect(ui.clip_rect());
    let playlist_drag =
        (row.dnd_hover_payload::<PlaylistDrag>().is_some() || files_hover) && group.len > 0;
    if playlist_drag && zone == DropZone::Before {
        fg.hline(rect.x_range(), rect.top(), line);
    } else if playlist_drag && zone == DropZone::AfterFolder {
        fg.hline(rect.x_range(), rect.bottom(), line);
    } else if hovered_drop {
        painter.rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.0, tokens.accent_text),
            egui::StrokeKind::Inside,
        );
    }
    let playing_here = current.is_some_and(|at| at >= group.start && at < group.start + group.len);
    let color = if playing_here {
        tokens.accent_text
    } else {
        tokens.text
    };
    let (x, mid) = (rect.left() + 10.0, rect.center().y);
    let text_x = rect.left() + 24.0 + GUTTER + digits as f32 * 8.0;
    let points = if collapsed {
        vec![
            egui::pos2(x - 3.0, mid - 4.5),
            egui::pos2(x - 3.0, mid + 4.5),
            egui::pos2(x + 4.0, mid),
        ]
    } else {
        vec![
            egui::pos2(x - 4.5, mid - 3.0),
            egui::pos2(x + 4.5, mid - 3.0),
            egui::pos2(x, mid + 4.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(
        points,
        tokens.text_muted,
        egui::Stroke::NONE,
    ));
    painter.text(
        egui::pos2(rect.left() + 16.0 + GUTTER + digits as f32 * 8.0, mid),
        egui::Align2::RIGHT_CENTER,
        format!("{number}."),
        egui::TextStyle::Body.resolve(ui.style()),
        tokens.text_muted,
    );
    if let Some((id, text, focused)) = renaming.as_mut().filter(|(id, ..)| *id == group.id) {
        let edit_id = Id::NULL.with(("media-group-edit", *id));
        let edit = ui.put(
            egui::Rect::from_min_max(
                egui::pos2(text_x - 2.0, rect.top() + 1.0),
                egui::pos2(rect.right() - 4.0, rect.bottom() - 1.0),
            ),
            egui::TextEdit::singleline(text)
                .id(edit_id)
                .char_limit(fm_media::MAX_GROUP_NAME),
        );
        edit.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Folder name"));
        if !*focused {
            edit.request_focus();
            *focused = true;
        }
        if edit.lost_focus() {
            let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let name = text.trim().to_string();
            if !cancelled && !name.is_empty() && name != group.name {
                actions.push(("media.group_rename", json!({"id": group.id, "name": name})));
            }
            *renaming = None;
        }
    } else {
        ui.painter().with_clip_rect(rect.with_min_x(text_x)).text(
            egui::pos2(text_x, mid),
            egui::Align2::LEFT_CENTER,
            &title,
            egui::TextStyle::Body.resolve(ui.style()),
            color,
        );
    }
    if group.locked {
        let at = egui::pos2(x + 17.0, mid);
        crate::media_icons::paint_lock(ui.painter(), at, tokens.text, tokens.panel);
        ui.interact(
            egui::Rect::from_center_size(at, egui::vec2(16.0, rect.height())),
            Id::NULL.with(("media-group-lock", group.id)),
            egui::Sense::hover(),
        )
        .on_hover_text("Locked: nothing can be dropped in or taken out. Right-click to unlock.");
    }
    if row.double_clicked() && group.len > 0 {
        actions.push(if can_continue {
            ("media.continue", json!({"index": group.start}))
        } else {
            ("media.play", json!({"index": group.start}))
        });
    }
    // Only the arrow collapses: a collapsing click would shift the header and break the double-click.
    let arrow = ui.interact(
        egui::Rect::from_min_size(rect.min, egui::vec2(22.0, rect.height())),
        Id::NULL.with(("media-group-arrow", group.id)),
        egui::Sense::click(),
    );
    let verb = if collapsed {
        "Expand folder"
    } else {
        "Collapse folder"
    };
    arrow.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, verb));
    if arrow.clicked() {
        *toggle = Some(group.id);
    }
    row.context_menu(|ui| {
        if ui
            .add_enabled(can_continue, egui::Button::new("Resume play"))
            .clicked()
        {
            actions.push(("media.continue", json!({"index": group.start})));
            ui.close();
        }
        if ui
            .add_enabled(group.len > 0, egui::Button::new("Play from start"))
            .clicked()
        {
            actions.push((
                "media.play",
                json!({"index": group.start, "from_start": true}),
            ));
            ui.close();
        }
        crate::media_book::menu_items(ui, book, group.start, actions);
        if ui.button("Rename…").clicked() {
            *renaming = Some((group.id, group.name.clone(), false));
            ui.close();
        }
        let lock = if group.locked {
            "Unlock folder"
        } else {
            "Lock folder"
        };
        if ui
            .button(lock)
            .on_hover_text(
                "A locked folder keeps its items: nothing can be dropped in or taken out",
            )
            .clicked()
        {
            actions.push(("media.group_lock", json!({"id": group.id})));
            ui.close();
        }
        ui.separator();
        if ui
            .button("Delete folder")
            .on_hover_text("Keeps its items")
            .clicked()
        {
            actions.push(("media.group_delete", json!({"id": group.id})));
            ui.close();
        }
        if ui
            .add_enabled(group.len > 0, egui::Button::new("Remove folder and items"))
            .clicked()
        {
            actions.push(("media.group_delete", json!({"id": group.id, "items": true})));
            ui.close();
        }
    });
}

/// Seek bar; the seek is sent when the drag ends so scrubbing does not flood the decoder.
fn seek_bar(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    width: f32,
    actions: &mut Vec<(&'static str, Value)>,
) {
    let media = &s.media;
    let book = media
        .book_view()
        .and_then(|view| view.total.map(|t| (t, view.position)));
    let duration = book.map_or_else(|| media.duration().unwrap_or(0.0), |(total, _)| total);
    let current = book.map_or(media.position, |(_, position)| position);
    let key = Id::NULL.with(("media-seek", pane));
    if let Some(target) = crate::player_controls::seek(ui, tokens, key, width, current, duration) {
        if book.is_some() {
            actions.push(("media.skip", json!({"by": target - current})));
        } else {
            actions.push(("media.seek", json!({"to": target})));
        }
    }
}

/// Speaker (mute toggle) and a short volume bar.
fn volume(ui: &mut Ui, s: &Session, tokens: &Tokens, actions: &mut Vec<(&'static str, Value)>) {
    crate::player_controls::volume(
        ui,
        tokens,
        s.media.volume,
        s.media.muted,
        "media.mute",
        "media.volume",
        actions,
    );
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
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            use crate::media_icons::{Icon, button};
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let on = |command| s.command_enabled(command);
                            if button(ui, tokens, Icon::Prev, "Previous", on("media.prev"), false)
                                .clicked()
                            {
                                actions.push(("media.prev", Value::Null));
                            }
                            let (icon, label) = if s.media.playing {
                                (Icon::Pause, "Pause")
                            } else {
                                (Icon::Play, "Play")
                            };
                            if button(ui, tokens, icon, label, on("media.toggle"), false).clicked()
                            {
                                actions.push(("media.toggle", Value::Null));
                            }
                            if button(ui, tokens, Icon::Next, "Next", on("media.next"), false)
                                .clicked()
                            {
                                actions.push(("media.next", Value::Null));
                            }
                            ui.label(tokens.muted(clock(s.media.position)));
                            seek_bar(ui, s, pane, tokens, 260.0, &mut actions);
                            ui.label(tokens.muted(clock(s.media.duration().unwrap_or(0.0))));
                        });
                        ui.horizontal(|ui| {
                            use crate::media_icons::{Icon, button};
                            volume(ui, s, tokens, &mut actions);
                            track_menus(ui, s, &mut actions);
                            ui.menu_button("Keys", keys_help);
                            if button(
                                ui,
                                tokens,
                                Icon::Fullscreen,
                                "Leave fullscreen (Esc or F)",
                                true,
                                true,
                            )
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
    crate::media_keys::note_slider_focus(ui.ctx(), false);
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
    use crate::media_icons::{Icon, button};
    let started = s.media.playing || (s.media.position > 0.0 && s.media.queue.current().is_some());
    if !started {
        return;
    }
    let mut actions: Vec<(&'static str, Value)> = Vec::new();
    let on = |command| s.command_enabled(command);
    if button(ui, tokens, Icon::Prev, "Previous", on("media.prev"), false).clicked() {
        actions.push(("media.prev", Value::Null));
    }
    let (icon, label) = if s.media.playing {
        (Icon::Pause, "Pause")
    } else {
        (Icon::Play, "Play")
    };
    if button(ui, tokens, icon, label, on("media.toggle"), false).clicked() {
        actions.push(("media.toggle", Value::Null));
    }
    if button(ui, tokens, Icon::Next, "Next", on("media.next"), false).clicked() {
        actions.push(("media.next", Value::Null));
    }
    let muted = s.media.muted;
    let (icon, label) = if muted {
        (Icon::Muted, "Volume (muted)")
    } else {
        (Icon::Volume, "Volume")
    };
    let speaker = button(ui, tokens, icon, label, true, muted);
    egui::Popup::from_toggle_button_response(&speaker)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let shown = if muted { 0.0 } else { s.media.volume };
            ui.vertical_centered(|ui| {
                ui.small(format!("{:.0}%", s.media.volume));
                const VOLUME: crate::media_audio::Scale = crate::media_audio::Scale {
                    min: 0.0,
                    max: 100.0,
                    origin: 0.0,
                    step: 1.0,
                    name: "Volume",
                    unit: "%",
                };
                if let Some(value) = crate::media_audio::vertical_slider(ui, shown, &VOLUME, 110.0)
                {
                    actions.push(("media.volume", json!({"value": value})));
                }
                let (icon, label) = if muted {
                    (Icon::Muted, "Unmute")
                } else {
                    (Icon::Volume, "Mute")
                };
                if button(ui, tokens, icon, label, true, muted).clicked() {
                    actions.push(("media.mute", json!({"on": !muted})));
                }
            });
        });
    for (command, params) in actions {
        s.dispatch_or_notice(command, &params);
    }
}

fn cover(ui: &mut Ui, pane: PaneId, image: Option<&Arc<fm_preview::ImageDoc>>, tokens: &Tokens) {
    let id = Id::NULL.with(("media-cover", pane));
    let Some(image) = image else {
        ui.data_mut(|d| d.remove::<(Arc<fm_preview::ImageDoc>, egui::TextureHandle)>(id));
        ui.allocate_ui(egui::vec2(72.0, 72.0), |ui| {
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
    let scale = 72.0 / image.width.max(image.height).max(1) as f32;
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

pub(crate) use crate::player_controls::clock;

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
        s.media
            .queue
            .group_range(0, 2, "music", Some("/music".into()))
            .expect("folder");
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
    fn book_controls_show_for_audiobooks_and_skip_by_the_preference() {
        let mut h = harness(Tokens::dark());
        assert!(h.query_by_label("Back 15 s").is_none());
        h.state_mut()
            .dispatch("media.enqueue", &json!({"uris": ["/music/solo.mp3"]}))
            .expect("enqueue");
        h.state_mut()
            .dispatch("media.book", &json!({"index": 2, "on": true}))
            .expect("mark");
        h.state_mut().media.queue.select(2).expect("select");
        h.state_mut().media.position = 100.0;
        h.run();
        h.get_by_label("Back 15 s").click();
        h.run();
        assert_eq!(
            h.state().media.position,
            100.0 - h.state().prefs.media_skip_back_secs as f64
        );
        h.state_mut()
            .dispatch("media.bookmark", &json!({"action": "add"}))
            .expect("bookmark");
        h.run();
        assert_eq!(
            h.state().media.book_view().map(|v| v.bookmarks.len()),
            Some(1)
        );
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
            h.get_by_label("Options").click();
            h.run();
            h.get_by_label("Keep playing when the pane is closed")
                .click();
            h.run();
            assert!(!h.state().prefs.media_background_audio);
            assert!(!h.state().media.playing);
            h.get_by_label("Clear playlist").click();
            h.run();
            assert!(h.state().media.queue.items().is_empty());
        }
    }

    #[test]
    fn folder_items_group_under_a_header_that_collapses() {
        let mut h = harness(Tokens::dark());
        h.get_by_label("Collapse folder").click();
        h.run();
        assert!(h.query_by_label("one").is_none());
        h.get_by_label("Expand folder").click();
        h.run();
        assert!(h.query_by_label("one").is_some());
    }

    #[test]
    fn new_folder_button_creates_an_empty_folder_that_files_can_be_moved_into() {
        let mut h = harness(Tokens::dark());
        h.get_by_label("New folder").click();
        h.run();
        h.run();
        let groups = h.state().media.queue.groups();
        assert_eq!(groups.len(), 2);
        assert_eq!((groups[1].name.as_str(), groups[1].len), ("New folder", 0));
        assert!(
            h.query_by_label_contains("empty, drop files here")
                .is_some()
        );
    }

    #[test]
    fn dragging_a_row_onto_another_places_it_after_that_row() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3"]}),
        )
        .expect("enqueue");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                },
                s,
            );
        let from = h.get_by_label("a").rect().center();
        let to = h.get_by_label("c").rect().center();
        h.hover_at(from);
        h.drag_at(from);
        h.run();
        h.hover_at(from + egui::vec2(0.0, 8.0));
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        h.run();
        let names: Vec<_> = h
            .state()
            .media
            .queue
            .items()
            .iter()
            .filter_map(|u| u.file_name().map(str::to_string))
            .collect();
        assert_eq!(names, ["b.mp3", "c.mp3", "a.mp3"]);
    }

    #[test]
    fn dropping_on_the_upper_half_of_the_first_row_puts_the_item_at_the_top() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3"]}),
        )
        .expect("enqueue");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                },
                s,
            );
        let from = h.get_by_label("c").rect().center();
        let first = h.get_by_label("a").rect();
        let to = egui::pos2(first.center().x, first.top() + 2.0);
        h.hover_at(from);
        h.drag_at(from);
        h.run();
        h.hover_at(from - egui::vec2(0.0, 8.0));
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        h.run();
        let names: Vec<_> = h
            .state()
            .media
            .queue
            .items()
            .iter()
            .filter_map(|u| u.file_name().map(str::to_string))
            .collect();
        assert_eq!(names, ["c.mp3", "a.mp3", "b.mp3"]);
    }

    #[test]
    fn dropping_on_the_top_of_a_folder_name_places_the_item_before_the_folder() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3", "/m/d.mp3"]}),
        )
        .expect("enqueue");
        s.media.queue.group_range(1, 2, "f", None).expect("group");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                },
                s,
            );
        let from = h.get_by_label("d").rect().center();
        let header = h.get_by_label("f · 2 files").rect();
        let to = egui::pos2(header.center().x, header.top() + 2.0);
        h.hover_at(from);
        h.drag_at(from);
        h.run();
        h.hover_at(from - egui::vec2(0.0, 8.0));
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        h.run();
        let names: Vec<_> = h
            .state()
            .media
            .queue
            .items()
            .iter()
            .filter_map(|u| u.file_name().map(str::to_string))
            .collect();
        assert_eq!(names, ["a.mp3", "d.mp3", "b.mp3", "c.mp3"]);
        let g = h.state().media.queue.groups();
        assert_eq!((g[0].start, g[0].len), (2, 2));
    }

    #[test]
    fn a_locked_folder_takes_no_drops_inside() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3", "/m/d.mp3"]}),
        )
        .expect("enqueue");
        let id = s.media.queue.group_range(1, 2, "f", None).expect("group");
        s.media.queue.set_group_locked(id, true).expect("lock");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                },
                s,
            );
        let from = h.get_by_label("d").rect().center();
        let header = h.get_by_label("f · 2 files").rect();
        let to = header.center() + egui::vec2(0.0, 3.0);
        h.hover_at(from);
        h.drag_at(from);
        h.run();
        h.hover_at(from - egui::vec2(0.0, 8.0));
        h.run();
        h.hover_at(to);
        h.run();
        h.drop_at(to);
        h.run();
        h.run();
        let g = h.state().media.queue.groups();
        assert_eq!((g[0].start, g[0].len), (1, 2));
        assert_eq!(h.state().media.queue.items().len(), 4);
    }

    #[test]
    fn ctrl_and_shift_click_select_several_rows_and_the_menu_removes_them() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3", "/m/d.mp3"]}),
        )
        .expect("enqueue");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                },
                s,
            );
        h.get_by_label("a").click();
        h.run();
        h.get_by_label("c").click_modifiers(egui::Modifiers::SHIFT);
        h.run();
        h.get_by_label("d")
            .click_modifiers(egui::Modifiers::COMMAND);
        h.run();
        h.get_by_label("b")
            .click_modifiers(egui::Modifiers::COMMAND);
        h.run();
        let pos = h.get_by_label("a").rect().center();
        h.hover_at(pos);
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run();
        h.run();
        h.get_by_label("Remove 3 items").click();
        h.run();
        let left: Vec<_> = h
            .state()
            .media
            .queue
            .items()
            .iter()
            .filter_map(|u| u.file_name().map(str::to_string))
            .collect();
        assert_eq!(left, ["b.mp3"]);
    }

    #[test]
    fn files_dropped_on_a_folder_row_join_that_folder() {
        let dir = tempfile::tempdir().expect("dir");
        let new = dir.path().join("new.mp3");
        std::fs::write(&new, b"x").expect("file");
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3"]}),
        )
        .expect("enqueue");
        s.media.queue.group_range(0, 2, "f", None).expect("group");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let uri = fm_vfs::VfsUri::parse(new.to_str().expect("utf8")).expect("uri");
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                    let rect = ui.max_rect();
                    crate::dnd::finish_media(ui.ctx(), s, rect, &tokens);
                },
                s,
            );
        h.run();
        let away = egui::pos2(300.0, 450.0);
        h.hover_at(away);
        h.drag_at(away);
        h.run();
        egui::DragAndDrop::set_payload(
            &h.ctx,
            crate::dnd::DragPayload {
                sources: vec![uri],
                origin: 1,
            },
        );
        let on = h.get_by_label("b").rect().center();
        h.hover_at(on);
        h.run();
        h.drop_at(on);
        h.run();
        for _ in 0..50 {
            h.state_mut().poll();
            h.run();
            if h.state().media.queue.items().len() == 4 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let g = h.state().media.queue.groups();
        assert_eq!((g.len(), g[0].len), (1, 3));
        assert_eq!(h.state().media.queue.items().len(), 4);
    }

    #[test]
    fn files_hovering_a_row_ask_to_land_after_it() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/m/a.mp3", "/m/b.mp3", "/m/c.mp3"]}),
        )
        .expect("enqueue");
        s.media.queue.group_range(0, 2, "f", None).expect("group");
        let pane = s.media_pane().expect("media");
        let tokens = Tokens::dark();
        let uri = fm_vfs::VfsUri::parse("/tmp/new.mp3").expect("uri");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        let mut h = Harness::builder()
            .with_size(egui::vec2(640.0, 480.0))
            .build_ui_state(
                move |ui, s| {
                    egui::CentralPanel::default().show(ui, |ui| show(ui, s, pane, &tokens));
                    let slot: Option<Value> = ui.ctx().data(|d| d.get_temp(drop_slot_id()));
                    if let Ok(mut v) = sink.lock() {
                        v.push(slot);
                    }
                },
                s,
            );
        h.run();
        let away = egui::pos2(300.0, 450.0);
        h.hover_at(away);
        h.drag_at(away);
        h.run();
        egui::DragAndDrop::set_payload(
            &h.ctx,
            crate::dnd::DragPayload {
                sources: vec![uri],
                origin: 1,
            },
        );
        h.hover_at(h.get_by_label("b").rect().center());
        h.run();
        h.run();
        assert!(
            seen.lock()
                .expect("lock")
                .contains(&Some(json!({"after": 1})))
        );
    }

    #[test]
    fn status_bar_pause_stops_playback() {
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
        h.get_by_label("Pause").click();
        h.run();
        assert!(!h.state().media.playing);
        assert_eq!(
            h.state().audio_owner,
            fm_engine::audio_owner::AudioOwner::None
        );
    }
}
