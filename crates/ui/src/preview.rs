//! Preview pane body: renders the engine's `Body` for the file under the cursor.
//! The UI only draws; every action is dispatched as a command (`preview.*`).

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{FontId, Id, RichText, ScrollArea, Sense, TextFormat, Ui, vec2};
use fm_engine::Session;
use fm_engine::preview_cmds::{Body, LogView, Zoom};
use fm_layout::{PaneId, PreviewSource};
use fm_preview::arrange::ThumbSort;
use fm_preview::log::{Level, LogDoc, LogRecord};
use fm_preview::{DirDoc, HexDoc, ImageDoc, MetaDoc, Preview, Span, StructDoc, TextDoc};
use serde_json::{Value, json};

use crate::theme::Tokens;

#[path = "preview_find.rs"]
mod find;
#[path = "preview_markdown.rs"]
mod markdown;
#[path = "preview_time.rs"]
mod time;

type Action = (&'static str, Value);

/// Per-pane text fields of the log filter bar.
#[derive(Clone)]
struct LogUi {
    filters_open: bool,
    query: String,
    regex: bool,
    time: Option<time::TimePicker>,
}

impl Default for LogUi {
    fn default() -> Self {
        Self {
            filters_open: true,
            query: String::new(),
            regex: false,
            time: None,
        }
    }
}

fn mono(tokens: &Tokens) -> FontId {
    FontId::monospace(tokens.font_size)
}

/// Preview content (thumbnails, pages) is much taller than a listing row, so egui's default
/// wheel step feels slow there.
const WHEEL_BOOST: f32 = 3.0;

fn boost_wheel(ui: &Ui) {
    let hovered = ui.rect_contains_pointer(ui.max_rect());
    if hovered {
        ui.input_mut(|i| {
            if !i.modifiers.command {
                i.smooth_scroll_delta *= WHEEL_BOOST;
            }
        });
    }
}

/// Render the preview pane body.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<fm_vfs::VfsUri> {
    boost_wheel(ui);
    fm_engine::preview_find_cmds::prepare(s, pane);
    let find_keyboard = s.layout.focused() == pane
        && !crate::dialogs::window_owns_keyboard(s)
        && ui.is_enabled()
        && ui.ctx().memory(|m| m.is_above_modal_layer(ui.layer_id()))
        && !ui.any_popup_open();
    if find_keyboard {
        for (modifiers, key, id) in [
            (egui::Modifiers::COMMAND, egui::Key::F, "preview.find.open"),
            (
                egui::Modifiers::SHIFT,
                egui::Key::F3,
                "preview.find.previous",
            ),
            (egui::Modifiers::NONE, egui::Key::F3, "preview.find.next"),
        ] {
            if s.command_enabled(id) && ui.input_mut(|i| i.consume_key(modifiers, key)) {
                s.dispatch_or_notice(id, &json!({"pane": pane}));
            }
        }
        if (s.preview_slot(pane).is_some_and(|slot| slot.find.open)
            || s.diffs_find(pane)
                .is_some_and(|f| f.open && s.pane_diff(pane).is_some_and(|d| d.active)))
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            s.dispatch_or_notice("preview.find.close", &json!({"pane": pane}));
            if s.preview_slot(pane)
                .is_some_and(|slot| matches!(slot.body, Body::Log(_)))
            {
                ui.data_mut(|d| {
                    let id = Id::NULL.with(("pv-logui", pane));
                    if let Some(mut state) = d.get_temp::<LogUi>(id) {
                        state.filters_open = false;
                        d.insert_temp(id, state);
                    }
                });
            }
            ui.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
    }
    let keyboard = s.layout.focused() == pane
        && !crate::dialogs::window_owns_keyboard(s)
        && !ui.ctx().text_edit_focused()
        && ui.is_enabled()
        && ui.ctx().memory(|m| m.is_above_modal_layer(ui.layer_id()))
        && !ui.any_popup_open();
    if keyboard {
        for (key, id) in [
            (egui::Key::Plus, "preview.scale.in"),
            (egui::Key::Equals, "preview.scale.in"),
            (egui::Key::Minus, "preview.scale.out"),
            (egui::Key::Num0, "preview.scale.reset"),
        ] {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                s.dispatch_or_notice(id, &json!({ "pane": pane }));
            }
        }
        if ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| ui.available_rect_before_wrap().contains(p))
        }) && ui
            .ctx()
            .pointer_hover_pos()
            .and_then(|p| ui.ctx().layer_id_at(p))
            .is_none_or(|layer| layer == ui.layer_id())
        {
            s.preview_text_selection = None;
        }
        if fm_engine::preview_text_cmds::selected(s, pane)
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            s.preview_text_selection = None;
        }
        if s.command_enabled("preview.text.select_all")
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A))
        {
            s.dispatch_or_notice("preview.text.select_all", &Value::Null);
        }
        if fm_engine::preview_text_cmds::selected(s, pane) {
            let copy = ui.input_mut(|i| {
                let mut copy = false;
                i.events.retain(|event| {
                    if matches!(event, egui::Event::Copy) {
                        copy = true;
                        false
                    } else {
                        true
                    }
                });
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::C) || copy
            });
            if copy {
                s.dispatch_or_notice("preview.text.copy", &Value::Null);
                ui.ctx().request_repaint();
            }
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(
            Id::NULL.with("pv-all-selected"),
            fm_engine::preview_text_cmds::selected(s, pane),
        )
    });
    let mut actions: Vec<Action> = Vec::new();
    let mut open: Option<fm_vfs::VfsUri> = None;
    let mut published: Option<fm_engine::file_cmds::GridSelection> = None;
    toolbar(ui, s, pane, &mut actions);
    find::bar(ui, s, pane, tokens, &mut actions);
    find::begin(ui, s, pane);
    ui.separator();
    let can_page = s.layout.focused() == pane
        && ui.is_enabled()
        && ui.ctx().memory(|m| m.is_above_modal_layer(ui.layer_id()))
        && !ui.ctx().text_edit_focused()
        && !ui.any_popup_open()
        && !ui.ctx().egui_is_using_pointer()
        && !ui
            .data(|d| d.get_temp::<LogUi>(Id::NULL.with(("pv-logui", pane))))
            .is_some_and(|st| st.time.is_some());
    ui.data_mut(|d| {
        d.insert_temp(Id::NULL.with("pv-can-page"), can_page);
        d.insert_temp(Id::NULL.with("pv-page-used"), false);
    });
    let scale = s.preview_slot(pane).map_or(1.0, |slot| slot.scale);
    let original_style = ui.style().clone();
    {
        let mut content_tokens = *tokens;
        content_tokens.font_size *= scale;
        content_tokens.row_height *= scale;
        content_tokens.padding *= scale;
        if scale != 1.0 {
            let style = ui.style_mut();
            for font in style.text_styles.values_mut() {
                font.size *= scale;
            }
            if let Some(font) = &mut style.override_font_id {
                font.size *= scale;
            }
            style.spacing.interact_size *= scale;
            style.spacing.item_spacing *= scale;
            style.spacing.button_padding *= scale;
        }
        let tokens = &content_tokens;
        ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-content-scale"), scale));
        if s.pane_diff(pane).is_some_and(|d| d.active) {
            crate::diff::show(ui, s, pane, tokens);
        } else if let Some(slot) = s.preview_slot(pane) {
            match &slot.body {
                Body::Empty => centered(ui, tokens, "Select a file to preview it"),
                Body::Loading => {
                    // Switching from a video keeps its layout, so the pane does not blink.
                    // A video gets its box (its size depends only on the pane) with the
                    // spinner inside, so the player does not blink while the file loads.
                    if slot.subject.as_ref().is_some_and(is_video_entry) {
                        let avail = ui.available_size();
                        let boxed = vec2(avail.x, video_box_height(ui, avail));
                        let (rect, _) = ui.allocate_exact_size(boxed, Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, tokens.bg);
                        ui.put(
                            egui::Rect::from_center_size(rect.center(), vec2(24.0, 24.0)),
                            egui::Spinner::new(),
                        );
                        reserve_controls(ui, pane, avail.x);
                    } else {
                        ui.centered_and_justified(|ui| ui.spinner());
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                Body::Static(p) if matches!(&**p, Preview::Dir(_)) => {
                    if let Preview::Dir(d) = &**p {
                        let volume = slot.subject.as_ref().and_then(|entry| {
                            s.volumes
                                .entries()
                                .iter()
                                .filter(|device| device.mount.is_same_or_descendant_of(&entry.uri))
                                .max_by_key(|device| device.mount.to_string().len())
                        });
                        if slot.thumbs {
                            if let Some(volume) = volume {
                                ui.push_id(("pv-volume", pane), |ui| {
                                    ui.collapsing("Volume information", |ui| {
                                        volume_card(ui, tokens, volume)
                                    });
                                });
                            }
                            thumb_grid(
                                ui,
                                s,
                                pane,
                                tokens,
                                d,
                                slot.thumb_size,
                                slot.thumb_sort,
                                slot.thumb_group,
                                slot.thumb_window,
                                &mut actions,
                                &mut open,
                                slot.subject.as_ref().map(|e| e.uri.clone()),
                                &mut published,
                            );
                        } else {
                            scrolled(ui, pane, "dir", |ui| {
                                if let Some(volume) = volume {
                                    volume_card(ui, tokens, volume);
                                }
                                dir_card(ui, tokens, d);
                            });
                        }
                    }
                }
                Body::Static(p) if slot.audio.is_some() => {
                    if let (Preview::Meta(meta), Some(audio)) = (&**p, &slot.audio) {
                        audio_body(ui, pane, tokens, p, meta, audio, &mut actions);
                    }
                }
                Body::Static(p)
                    if slot.video.is_none()
                        && matches!(&**p, Preview::Meta(m) if m.video.is_some()) =>
                {
                    if let Preview::Meta(m) = &**p
                        && let Some(info) = &m.video
                    {
                        pending_video_body(ui, pane, tokens, m, info);
                    }
                }
                Body::Static(p) if slot.video.is_some() => {
                    if let (Preview::Meta(m), Some(v)) = (&**p, &slot.video) {
                        video_body(ui, pane, tokens, p, m, v, &mut actions);
                    }
                }
                Body::Static(p) => {
                    let p = p.clone();
                    let mut link = None;
                    let view = View {
                        pretty: slot.pretty,
                        subject: slot.subject.as_ref().map(|e| &e.uri),
                        source: slot.source,
                        wrap: slot.wrap,
                        zoom: slot.zoom,
                    };
                    let images = match (&*p, slot.source, slot.subject.as_ref()) {
                        (Preview::Markdown(m), false, Some(document)) => Some(
                            markdown::image_urls(m, tokens.dark)
                                .into_iter()
                                .map(|url| {
                                    let state = s.md_images.state(
                                        document,
                                        &url,
                                        s.prefs.preview_remote_images,
                                    );
                                    (url, state)
                                })
                                .collect(),
                        ),
                        _ => None,
                    };
                    let mut want = Vec::new();
                    if let Some(block) =
                        static_body(ui, pane, tokens, &p, view, &mut link, images, &mut want)
                    {
                        actions.push(("preview.code.copy", json!({ "block": block })));
                    }
                    if !want.is_empty() {
                        actions.push((
                            "preview.md_images.want",
                            json!({ "urls": want, "pane": pane }),
                        ));
                    }
                    if let Some(url) = link {
                        actions.push(("preview.open_link", json!({ "url": url, "pane": pane })));
                    }
                }
                Body::Grid(items) => multi_grid(ui, pane, tokens, items, &mut actions),
                Body::Log(v) => {
                    log_body(ui, pane, tokens, v, &mut actions);
                    let id = Id::NULL.with(("pv-logui", pane));
                    let mut st: LogUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
                    if let Some(picker) = &mut st.time {
                        picker.show(ui.ctx(), pane, tokens, &mut actions);
                        if !picker.open {
                            st.time = None;
                        }
                    }
                    ui.data_mut(|d| d.insert_temp(id, st));
                }
            }
        }
    }
    ui.set_style(original_style);
    find::end(ui, s, pane);
    if s.preview_slot(pane).is_some_and(|slot| slot.find.busy) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(30));
    }
    ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-content-scale"), 1.0_f32));
    ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-can-page"), false));
    ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-all-selected"), false));
    for (id, params) in actions {
        let mut params = params;
        if let Some(o) = params.as_object_mut() {
            o.insert("pane".into(), json!(pane));
        }
        s.dispatch_or_notice(id, &params);
        ui.ctx().request_repaint();
    }
    match published {
        Some(g) => s.grid_selection = Some(g),
        None if s.grid_selection.as_ref().is_some_and(|g| g.pane == pane) => {
            s.grid_selection = None;
        }
        None => {}
    }
    open
}

fn centered(ui: &mut Ui, tokens: &Tokens, msg: &str) {
    ui.centered_and_justified(|ui| ui.label(tokens.muted(msg)));
}

fn paged_scroll(
    ui: &mut Ui,
    area: ScrollArea,
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> ScrollArea {
    // ScrollArea prehashes its salt before passing it to make_persistent_id.
    let id = ui.make_persistent_id(egui::IdSalt::new(&salt));
    let enabled = ui.data_mut(|d| {
        let enabled = d
            .get_temp::<bool>(Id::NULL.with("pv-can-page"))
            .unwrap_or(false);
        d.insert_temp(Id::NULL.with("pv-can-page"), false);
        enabled
    });
    let (pages, lines, horizontal, home, end) = if enabled {
        ui.input_mut(|input| {
            let down = input.consume_key(egui::Modifiers::NONE, egui::Key::PageDown);
            let up = input.consume_key(egui::Modifiers::NONE, egui::Key::PageUp);
            let line_down = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            let line_up = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let right = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
            let left = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            let home = input.consume_key(egui::Modifiers::NONE, egui::Key::Home);
            let end = input.consume_key(egui::Modifiers::NONE, egui::Key::End);
            (
                i32::from(down) - i32::from(up),
                i32::from(line_down) - i32::from(line_up),
                i32::from(right) - i32::from(left),
                home,
                end,
            )
        })
    } else {
        (0, 0, 0, false, false)
    };
    let area = area.id_salt(salt);
    if pages == 0 && lines == 0 && horizontal == 0 && !home && !end {
        return area;
    }
    ui.data_mut(|d| {
        d.insert_temp(
            Id::NULL.with("pv-page-used"),
            pages != 0 || lines != 0 || home || end,
        )
    });
    let offset = egui::scroll_area::State::load(ui.ctx(), id)
        .map_or(egui::Vec2::ZERO, |state| state.clamped_offset());
    let page = ui.available_height().max(1.0) * 0.9;
    let line = ui.text_style_height(&egui::TextStyle::Monospace);
    let bottom = ui
        .data(|d| d.get_temp::<f32>(id.with("pv-bottom")))
        .unwrap_or(0.0);
    let y = if home {
        0.0
    } else if end {
        bottom
    } else {
        (offset.y + pages as f32 * page + lines as f32 * line).max(0.0)
    };
    let area = area.horizontal_scroll_offset((offset.x + horizontal as f32 * line).max(0.0));
    if pages != 0 || lines != 0 || home || end {
        area.stick_to_bottom(false).vertical_scroll_offset(y)
    } else {
        area
    }
}

fn remember_scroll<R>(ui: &Ui, output: &egui::scroll_area::ScrollAreaOutput<R>) {
    let bottom = (output.content_size.y - output.inner_rect.height()).max(0.0);
    ui.data_mut(|d| d.insert_temp(output.id.with("pv-bottom"), bottom));
}

fn toolbar(ui: &mut Ui, s: &Session, pane: PaneId, actions: &mut Vec<Action>) {
    let Some(slot) = s.preview_slot(pane) else {
        return;
    };
    // The view options go in a closure so they can be measured, then shown inline or, when the
    // pane is too narrow, behind a menu button on the right.
    let diff = s.pane_diff(pane);
    let options = |ui: &mut Ui, actions: &mut Vec<Action>| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().button_padding = vec2(10.0, 3.0);
        // The diff's own controls live in its second bar.
        if diff.is_some_and(|d| d.active) {
            return;
        }
        if !matches!(slot.body, Body::Log(_)) {
            let can_find = matches!(&slot.body, Body::Static(p) if fm_engine::preview_find_cmds::searchable(p));
            if ui
                .add_enabled(can_find, egui::Button::new("Find"))
                .on_hover_text("Find in loaded text (Ctrl+F)")
                .on_disabled_hover_text(
                    "Find is available for text, structured and Markdown previews.",
                )
                .clicked()
            {
                actions.push(("preview.find.open", json!({})));
            }
        }
        if ui
            .button("−")
            .on_hover_text("Zoom preview out (Ctrl+-)")
            .clicked()
        {
            actions.push(("preview.scale.out", json!({})));
        }
        let reset = ui
            .button(format!("{:.0}%", slot.scale * 100.0))
            .on_hover_text("Reset preview zoom (Ctrl+0)");
        reset.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, "Reset preview zoom")
        });
        if reset.clicked() {
            actions.push(("preview.scale.reset", json!({})));
        }
        if ui
            .button("+")
            .on_hover_text("Zoom preview in (Ctrl+=)")
            .clicked()
        {
            actions.push(("preview.scale.in", json!({})));
        }
        if slot.updates_available && ui.button("Reload")
            .on_hover_text("Selected files changed. Reload their preview while keeping filters and source order.")
            .clicked()
        {
            actions.push(("preview.reload", json!({})));
        }
        let wrap_pretty = |ui: &mut Ui, actions: &mut Vec<Action>| {
            if ui
                .add(egui::Button::selectable(slot.wrap, "Wrap"))
                .clicked()
            {
                actions.push(("preview.toggle_wrap", json!({})));
            }
        };
        if let Body::Static(p) = &slot.body {
            match &**p {
                Preview::Structured(d) => {
                    let ok = d.pretty.is_ok();
                    let r = ui.add_enabled(ok, egui::Button::selectable(slot.pretty, "Pretty"));
                    let r = match &d.pretty {
                        Err(e) => r.on_disabled_hover_text(e),
                        Ok(_) => r,
                    };
                    if r.clicked() {
                        actions.push(("preview.toggle_pretty", json!({})));
                    }
                    wrap_pretty(ui, actions);
                }
                Preview::Text(_) => wrap_pretty(ui, actions),
                Preview::Markdown(_) => {
                    // The git Preview button already toggles the source when it is shown.
                    if !diff.is_some_and(|d| !d.is_pair())
                        && ui
                            .add(egui::Button::selectable(!slot.source, "Preview"))
                            .on_hover_text(markdown_toggle_hint(slot.source))
                            .clicked()
                    {
                        actions.push(("preview.toggle_source", json!({})));
                    }
                    wrap_pretty(ui, actions);
                }
                Preview::Image(_) => {
                    for (label, mode) in [("Fit", "fit"), ("100%", "actual")] {
                        if ui.button(label).clicked() {
                            actions.push(("preview.zoom", json!({ "mode": mode })));
                        }
                    }
                }
                Preview::Dir(_) => thumb_toolbar(
                    ui,
                    pane,
                    slot.thumbs,
                    slot.thumb_size,
                    slot.thumb_sort,
                    slot.thumb_group,
                    s.prefs.recursive_preview,
                    actions,
                ),
                _ => {}
            }
        }
    };
    ui.horizontal(|ui| {
        source_picker(ui, s, pane, actions);
        let plain_text = !matches!(&slot.body, Body::Static(p) if matches!(&**p, Preview::Markdown(_)));
        if let Some(d) = diff.filter(|d| d.is_pair() || plain_text) {
            // Two files, or anything but Markdown (the only view with a source toggle): one compact toggle.
            let resp = ui
                .add(egui::Button::selectable(d.active, "Diff"))
                .on_hover_text(if d.active {
                    "Back to the file preview"
                } else if d.is_pair() {
                    "Compare the two files"
                } else {
                    "Show the changes against the last commit"
                });
            if resp.clicked() {
                actions.push(("preview.diff.toggle", json!({ "show": !d.active })));
            }
        } else if let Some(d) = diff {
            let markdown = matches!(&slot.body, Body::Static(p) if matches!(&**p, Preview::Markdown(_)));
            for (label, show) in [("Preview", false), ("Diff", true)] {
                let rendered = markdown && !slot.source;
                let selected = d.active == show && (show || !markdown || rendered);
                let resp = ui
                    .add(egui::Button::selectable(selected, label))
                    .on_hover_text(if show {
                        "Show the changes against the last commit"
                    } else if markdown {
                        markdown_toggle_hint(slot.source)
                    } else {
                        "Show the file"
                    });
                if resp.clicked() {
                    if d.active != show {
                        actions.push(("preview.diff.toggle", json!({ "show": show })));
                    } else if !show && markdown {
                        actions.push(("preview.toggle_source", json!({})));
                    }
                }
            }
        }
        if matches!(slot.body, Body::Log(_)) || slot.subjects.len() > 1 {
            let id = Id::NULL.with(("pv-logui", pane));
            let mut st: LogUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
            let focus_id = Id::NULL.with(("pv-log-find-focus", pane));
            if matches!(slot.body, Body::Log(_)) && slot.find.open
                && ui.data(|d| d.get_temp::<u64>(focus_id)) != Some(slot.find.focus)
            {
                st.filters_open = true;
                ui.data_mut(|d| {
                    d.insert_temp(focus_id, slot.find.focus);
                    d.insert_temp(Id::NULL.with(("pv-log-request-focus", pane)), true);
                });
            }
            if matches!(slot.body, Body::Log(_)) && ui
                    .add(egui::Button::selectable(st.filters_open, RichText::new("Filter").strong()).frame_when_inactive(true))
                    .on_hover_text(
                        "Show or hide log filters, including search (Ctrl+F). Applied filters stay active when hidden.",
                    )
                    .clicked()
                {
                    st.filters_open = !st.filters_open;
                }
            ui.data_mut(|d| d.insert_temp(id, st));
        }
        let needed = {
            let rect =
                egui::Rect::from_min_size(ui.cursor().min, vec2(10_000.0, ui.available_height()));
            let mut sizing = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                    .sizing_pass()
                    .invisible(),
            );
            options(&mut sizing, &mut Vec::new());
            sizing.min_rect().width() + 24.0
        };
        let inline = needed + 60.0 <= ui.available_width();
        let controls = if inline { needed } else { 36.0 };
        let (title, folder) = if slot.subjects.len() > 1 {
            (format!("{} selected files", slot.subjects.len()), None)
        } else {
            slot.subject.as_ref().map_or_else(
                || (String::new(), None),
                subject_title,
            )
        };
        let title_width = (ui.available_width() - controls - ui.spacing().item_spacing.x).max(0.0);
        toolbar_title(ui, &title, folder.as_deref(), title_width);
        if inline {
            // Framed buttons so view options stand apart from the title.
            ui.add_space(12.0);
            ui.separator();
            options(ui, actions);
        } else if needed > 24.0 {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = egui::Button::new("").min_size(vec2(28.0, 0.0));
                let (response, _) = egui::containers::menu::MenuButton::from_button(button)
                    .ui(ui, |ui| options(ui, actions));
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "View options"));
                // The bundled fonts have no hamburger glyph: draw it.
                let stroke = ui.style().interact(&response).fg_stroke;
                let c = response.rect.center();
                for dy in [-4.0, 0.0, 4.0] {
                    ui.painter().hline(c.x - 7.0..=c.x + 7.0, c.y + dy, stroke);
                }
                response.on_hover_text("View options");
            });
        }
    });
    if let Body::Log(v) = &slot.body
        && ui
            .data(|d| d.get_temp::<LogUi>(Id::NULL.with(("pv-logui", pane))))
            .is_some_and(|st| st.filters_open)
    {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.spacing_mut().button_padding = vec2(10.0, 3.0);
            log_toolbar(ui, pane, v, actions);
        });
    }
}

fn subject_title(entry: &fm_vfs::Entry) -> (String, Option<String>) {
    let folder = if entry.is_dir_like() {
        Some(entry.uri.clone())
    } else {
        entry.uri.parent()
    };
    let title = if entry.name == "." {
        String::new()
    } else {
        entry.display_name().to_owned()
    };
    (
        title,
        folder.map(|uri| uri.local_path_string().unwrap_or_else(|| uri.to_string())),
    )
}

fn toolbar_title(ui: &mut Ui, title: &str, folder: Option<&str>, width: f32) {
    let height = ui.spacing().interact_size.y;
    let Some(folder) = folder else {
        ui.add_sized(
            vec2(width, height),
            egui::Label::new(RichText::new(title).strong()).truncate(),
        )
        .on_hover_text(title);
        return;
    };
    let measure = |text: &str| {
        egui::WidgetText::from(text)
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Body,
            )
            .size()
            .x
    };
    let gap = if title.is_empty() {
        0.0
    } else {
        ui.spacing().item_spacing.x.min(width)
    };
    // Keep the folder intact until the name has shrunk to an ellipsis.
    let min_name = if title.is_empty() {
        0.0
    } else {
        measure("…")
    };
    let folder_width = measure(folder).min((width - gap - min_name).max(0.0));
    let name_width = (width - gap - folder_width).max(0.0);
    let displayed_folder = middle_ellipsis(folder, folder_width, measure);
    ui.allocate_ui_with_layout(
        vec2(width, height),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if !title.is_empty() {
                ui.add_sized(
                    vec2(name_width, height),
                    egui::Label::new(RichText::new(title).strong()).truncate(),
                )
                .on_hover_text(title);
            }
            ui.add_sized(
                vec2(folder_width, height),
                egui::Label::new(RichText::new(displayed_folder).weak()).truncate(),
            )
            .on_hover_text(folder);
        },
    );
}

fn middle_ellipsis(text: &str, width: f32, measure: impl Fn(&str) -> f32) -> String {
    if measure(text) <= width {
        return text.to_owned();
    }
    if measure("…") > width {
        return String::new();
    }
    let chars = text.chars().count();
    let abbreviated = |keep: usize| {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let mut result = String::with_capacity(text.len() + '…'.len_utf8());
        result.extend(text.chars().take(head));
        result.push('…');
        result.extend(text.chars().skip(chars - tail));
        result
    };
    let mut low = 0;
    let mut high = chars.saturating_sub(1);
    while low < high {
        let keep = low + (high - low).div_ceil(2);
        if measure(&abbreviated(keep)) <= width {
            low = keep;
        } else {
            high = keep - 1;
        }
    }
    abbreviated(low)
}

fn source_picker(ui: &mut Ui, s: &Session, pane: PaneId, actions: &mut Vec<Action>) {
    let Some(state) = s.layout.state(pane) else {
        return;
    };
    let files: Vec<PaneId> = s
        .layout
        .panes()
        .into_iter()
        .filter(|p| s.layout.state(*p).is_some_and(|st| st.kind.is_files()))
        .collect();
    // Pane ids mean nothing to users: number the file panes; while the list is open each pane
    // shows the same number as an overlay.
    let number = |id: PaneId| files.iter().position(|p| *p == id).map_or(0, |i| i + 1);
    let current = match state.preview_source {
        PreviewSource::Follow => "Active pane".to_string(),
        PreviewSource::Pane(id) => format!("Pane {}", number(id)),
    };
    let response = egui::ComboBox::from_id_salt(("pv-source", pane))
        .selected_text(current)
        .width(100.0)
        .show_ui(ui, |ui| {
            let ctx = ui.ctx();
            let pass = ctx.cumulative_pass_nr();
            ctx.data_mut(|d| d.insert_temp(crate::pane::source_open_id(), pass));
            if ui
                .selectable_label(
                    state.preview_source == PreviewSource::Follow,
                    "Active pane (follows focus)",
                )
                .clicked()
            {
                actions.push(("pane.preview.source", json!({ "source": "focus" })));
            }
            for id in files.iter().copied() {
                let on = state.preview_source == PreviewSource::Pane(id);
                let label = if s.layout.focused() == id {
                    format!("Pane {} (focused)", number(id))
                } else {
                    format!("Pane {}", number(id))
                };
                if ui.selectable_label(on, label).clicked() {
                    actions.push(("pane.preview.source", json!({ "source": id })));
                }
            }
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ComboBox, response.enabled(), "Preview source")
    });
}

fn markdown_toggle_hint(source: bool) -> &'static str {
    if source {
        "Showing the Markdown text. Click to show the rendered document."
    } else {
        "Click to show the Markdown text instead of the rendered document."
    }
}

/// How a static body is shown.
#[derive(Clone, Copy)]
struct View<'a> {
    pretty: bool,
    /// The file being shown, when it is a single file (the target of "Copy Image").
    subject: Option<&'a fm_vfs::VfsUri>,
    source: bool,
    wrap: bool,
    zoom: Zoom,
}

#[allow(clippy::too_many_arguments)]
fn static_body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    p: &Arc<Preview>,
    View {
        pretty,
        subject,
        source,
        wrap,
        zoom,
    }: View,
    link: &mut Option<String>,
    images: Option<markdown::ImageStates>,
    want: &mut Vec<String>,
) -> Option<usize> {
    match &**p {
        Preview::Text(d) => text_view(ui, pane, tokens, d, wrap),
        Preview::Structured(d) => structured(ui, pane, tokens, d, pretty, wrap),
        Preview::Markdown(m) if source => text_view(ui, pane, tokens, &m.source, wrap),
        Preview::Markdown(m) => {
            return markdown::show(ui, pane, tokens, m, wrap, link, images, want);
        }
        Preview::Image(i) => image(ui, pane, tokens, p, i, zoom, subject),
        Preview::Hex(h) => hex(ui, tokens, h),
        Preview::Dir(d) => scrolled(ui, pane, "dir", |ui| dir_card(ui, tokens, d)),
        Preview::Meta(m) => scrolled(ui, pane, "meta", |ui| {
            meta_card(ui, tokens, m);
            if let Some(poster) = &m.poster {
                image(ui, pane, tokens, p, poster, Zoom::Fit, None);
            }
        }),
        Preview::Error(e) => scrolled(ui, pane, "err", |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Cannot preview").color(tokens.error).strong());
            ui.label(e);
        }),
        Preview::Log(doc) => {
            let row_h = line_height(tokens);
            tight_rows(ui);
            let output = paged_scroll(ui, ScrollArea::both(), ("pv-log-card", pane)).show_rows(
                ui,
                row_h,
                doc.records.len(),
                |ui, range| {
                    tight_rows(ui);
                    for index in range {
                        if let Some(record) = doc.records.get(index) {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(record.first_line())
                                        .font(mono(tokens))
                                        .line_height(Some(row_h)),
                                )
                                .extend(),
                            );
                        }
                    }
                },
            );
            remember_scroll(ui, &output);
        }
    }
    None
}

fn multi_grid(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    items: &[(fm_vfs::Entry, Arc<Preview>)],
    actions: &mut Vec<Action>,
) {
    let scale = content_scale(ui);
    let columns = ((ui.available_width() / (300.0 * scale)).floor() as usize).clamp(1, 4);
    let output = paged_scroll(ui, ScrollArea::vertical(), ("pv-multi-grid", pane)).show(ui, |ui| {
        for (row_index, row) in items.chunks(columns).enumerate() {
            ui.columns(columns, |uis| {
                for (column_index, (column, (entry, preview))) in uis.iter_mut().zip(row).enumerate() {
                    column.push_id(&entry.uri, |ui| {
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&entry.name).strong()).truncate(),
                            )
                            .on_hover_text(entry.uri.to_string());
                            ui.separator();
                            ui.allocate_ui(vec2(ui.available_width(), 240.0 * scale), |ui| {
                                ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                                let mut link = None;
                                if let Some(block) = static_body(
                                    ui,
                                    pane,
                                    tokens,
                                    preview,
                                    View { pretty: false, subject: Some(&entry.uri), source: false, wrap: false, zoom: Zoom::Fit },
                                    &mut link,
                                    None,
                                    &mut Vec::new(),
                                ) {
                                    actions.push(("preview.code.copy", json!({ "block": block, "item": row_index * columns + column_index })));
                                }
                                if let Some(url) = link {
                                    actions.push(("preview.open_link", json!({ "url": url, "pane": pane })));
                                }
                            });
                        });
                    });
                }
            });
            ui.add_space(tokens.padding);
        }
    });
    remember_scroll(ui, &output);
}

/// Cards are taller than a small pane; scroll them instead of overflowing the slot.
fn scrolled(ui: &mut Ui, pane: PaneId, what: &str, add: impl FnOnce(&mut Ui)) {
    let output = paged_scroll(ui, ScrollArea::vertical(), ("pv-scroll", what, pane))
        .auto_shrink(false)
        // Scroll content shares its parent's id; scope it so side-by-side panes don't collide.
        .show(ui, |ui| {
            ui.push_id((what, pane), add);
        });
    remember_scroll(ui, &output);
}

fn spans_job(tokens: &Tokens, spans: &[Span], wrap: bool, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for sp in spans {
        let mut format = TextFormat::simple(mono(tokens), tokens.syntax(sp.class));
        format.line_height = Some(line_height(tokens));
        job.append(&sp.text, 0.0, format);
    }
    job.wrap.max_width = if wrap { width } else { f32::INFINITY };
    job
}

fn selection_job(ui: &Ui, tokens: &Tokens, mut job: LayoutJob) -> LayoutJob {
    if ui
        .data(|d| d.get_temp::<bool>(Id::NULL.with("pv-all-selected")))
        .unwrap_or(false)
    {
        for section in &mut job.sections {
            section.format.background = tokens.selection_bg;
            section.format.color = tokens.selection_text;
        }
    }
    job
}

/// Row height of the text preview: the font plus some air, built into each line's galley.
fn line_height(tokens: &Tokens) -> f32 {
    (tokens.font_size * 1.45).round()
}

/// Lines sit flush against each other: egui's copy inserts a blank line wherever the gap
/// between two labels exceeds half a row.
fn tight_rows(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.spacing_mut().interact_size.y = 0.0;
}

fn text_view(ui: &mut Ui, pane: PaneId, tokens: &Tokens, d: &TextDoc, wrap: bool) {
    if d.truncated {
        ui.label(tokens.muted(format!(
            "Showing the first {} lines of {} bytes",
            d.lines.len(),
            d.total_bytes
        )));
    }
    let gutter = d.lines.len().to_string().len();
    let row_h = line_height(tokens);
    // `show_rows` sizes the scrollable height from the spacing of the Ui it is called on, so the
    // tight spacing must be set here, not only inside the rows.
    tight_rows(ui);
    let area = if wrap {
        ScrollArea::vertical()
    } else {
        ScrollArea::both()
    };
    let area = paged_scroll(ui, area, ("pv-text", pane)).auto_shrink(false);
    let area = if !wrap && let Some(line) = find::target_line(ui) {
        area.vertical_scroll_offset(line as f32 * row_h)
    } else {
        area
    };
    let output = if wrap {
        area.show(ui, |ui| {
            tight_rows(ui);
            for (i, line) in d.lines.iter().enumerate() {
                find::set_segment(ui, i);
                ui.horizontal_top(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{:>gutter$}", i + 1))
                                .font(mono(tokens))
                                .line_height(Some(row_h))
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                    find::label(
                        ui,
                        tokens,
                        selection_job(ui, tokens, spans_job(tokens, line, true, 0.0)),
                        true,
                    );
                });
            }
        })
    } else {
        area.show_rows(ui, row_h, d.lines.len(), |ui, range| {
            tight_rows(ui);
            for i in range {
                let Some(line) = d.lines.get(i) else { continue };
                find::set_segment(ui, i);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{:>gutter$}", i + 1))
                                .font(mono(tokens))
                                .line_height(Some(row_h))
                                .color(tokens.text_muted),
                        )
                        .selectable(false),
                    );
                    find::label(
                        ui,
                        tokens,
                        selection_job(ui, tokens, spans_job(tokens, line, false, 0.0)),
                        false,
                    );
                });
            }
        })
    };
    remember_scroll(ui, &output);
}

fn structured(ui: &mut Ui, pane: PaneId, tokens: &Tokens, d: &StructDoc, pretty: bool, wrap: bool) {
    if pretty && let Some(n) = &d.pretty_note {
        ui.label(tokens.muted(n));
    }
    let doc = match (&d.pretty, pretty) {
        (Ok(p), true) => p,
        _ => &d.raw,
    };
    text_view(ui, pane, tokens, doc, wrap);
}

fn image(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    p: &Arc<Preview>,
    img: &ImageDoc,
    zoom: Zoom,
    subject: Option<&fm_vfs::VfsUri>,
) {
    if img.frames.is_empty() {
        centered(ui, tokens, "No pixels to show");
        return;
    }
    let ctx = ui.ctx().clone();
    let id = ui.make_persistent_id(("pv-tex", pane));
    let ident = Arc::as_ptr(p) as usize;
    let cached: Option<(usize, Vec<egui::TextureHandle>)> = ctx.data_mut(|d| d.get_temp(id));
    let textures = match cached {
        Some((k, t)) if k == ident => t,
        _ => {
            let t: Vec<egui::TextureHandle> = img
                .frames
                .iter()
                .enumerate()
                .map(|(i, (rgba, _))| {
                    let ci = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width as usize, img.height as usize],
                        rgba,
                    );
                    ctx.load_texture(format!("pv-{pane}-{i}"), ci, egui::TextureOptions::LINEAR)
                })
                .collect();
            ctx.data_mut(|d| d.insert_temp(id, (ident, t.clone())));
            t
        }
    };
    let mut frame = 0;
    if textures.len() > 1 {
        let total: u32 = img.frames.iter().map(|f| f.1.max(20)).sum();
        let t = (ctx.input(|i| i.time) * 1000.0) as u64 % u64::from(total.max(1));
        let mut acc = 0u64;
        for (i, f) in img.frames.iter().enumerate() {
            acc += u64::from(f.1.max(20));
            if t < acc {
                frame = i;
                break;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
    let Some(tex) = textures.get(frame) else {
        return;
    };
    let (ow, oh) = img.original;
    let avail = ui.available_size() - vec2(0.0, 22.0);
    let natural = vec2(ow.max(1) as f32, oh.max(1) as f32);
    let scale = content_scale(ui)
        * match zoom {
            Zoom::Fit => (avail.x / natural.x)
                .min(avail.y / natural.y)
                .clamp(0.01, 1.0),
            Zoom::Actual => 1.0,
            Zoom::Scale(v) => v,
        };
    let output = paged_scroll(ui, ScrollArea::both(), ("pv-img", pane))
        .auto_shrink(false)
        .max_height(avail.y)
        .show(ui, |ui| {
            let response =
                ui.add(egui::Image::new((tex.id(), natural * scale)).sense(Sense::click()));
            if let Some(uri) = subject {
                response.context_menu(|ui| crate::context_menu::copy_image_item(ui, uri));
            }
        });
    remember_scroll(ui, &output);
    ui.label(tokens.muted(format!(
        "{} · {ow}×{oh} · {:.0}%{}",
        img.format,
        scale * 100.0,
        if (img.width, img.height) != img.original {
            " (preview downscaled)"
        } else {
            ""
        }
    )));
}

fn content_scale(ui: &Ui) -> f32 {
    ui.data(|d| d.get_temp::<f32>(Id::NULL.with("pv-content-scale")))
        .unwrap_or(1.0)
}

fn hex(ui: &mut Ui, tokens: &Tokens, h: &HexDoc) {
    let row_h = ui.fonts_mut(|f| f.row_height(&mono(tokens))) + 2.0;
    let rows = h.bytes.len().div_ceil(16);
    let output = paged_scroll(ui, ScrollArea::both(), "pv-hex")
        .auto_shrink(false)
        .show_rows(ui, row_h, rows, |ui, range| {
            for r in range {
                let chunk = h
                    .bytes
                    .get(r * 16..((r + 1) * 16).min(h.bytes.len()))
                    .unwrap_or(&[]);
                let hexs: String = (0..16)
                    .map(|i| {
                        chunk
                            .get(i)
                            .map_or("   ".to_string(), |b| format!("{b:02x} "))
                    })
                    .collect();
                let ascii: String = chunk
                    .iter()
                    .map(|b| {
                        if (0x20..0x7f).contains(b) {
                            *b as char
                        } else {
                            '.'
                        }
                    })
                    .collect();
                ui.label(
                    RichText::new(format!("{:08x}  {hexs} {ascii}", r * 16)).font(mono(tokens)),
                );
            }
        });
    remember_scroll(ui, &output);
    if (h.bytes.len() as u64) < h.total {
        ui.label(tokens.muted(format!("First {} of {} bytes", h.bytes.len(), h.total)));
    }
}

fn card_rows(ui: &mut Ui, tokens: &Tokens, id: &str, rows: &[(String, String)]) {
    let label_width = (ui.available_width() * 0.3).clamp(80.0, 140.0);
    let value_width = (ui.available_width() - label_width - 16.0).max(20.0);
    egui::Grid::new(id)
        .num_columns(2)
        .min_row_height(tokens.font_size + 4.0)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                let height = |text: &str, width| {
                    ui.painter()
                        .layout(
                            text.to_owned(),
                            FontId::proportional(tokens.font_size),
                            tokens.text,
                            width,
                        )
                        .size()
                        .y
                };
                let row_height = height(k, label_width).max(height(v, value_width));
                ui.allocate_ui_with_layout(
                    vec2(label_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add(egui::Label::new(RichText::new(k).color(tokens.text_muted)).wrap());
                    },
                );
                ui.allocate_ui_with_layout(
                    vec2(value_width, row_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.add(egui::Label::new(v).wrap().selectable(true));
                    },
                );
                ui.end_row();
            }
        });
}

fn information(ui: &mut Ui, tokens: &Tokens, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.scope(|ui| {
        crate::dialog_style::apply(ui, tokens);
        ui.spacing_mut().interact_size.y = tokens.font_size + 4.0;
        ui.spacing_mut().item_spacing.y = 6.0;
        egui::Frame::NONE
            .inner_margin(tokens.padding)
            .show(ui, |ui| {
                crate::dialog_style::section_heading(ui, tokens, title);
                body(ui);
            });
    });
}

fn volume_card(ui: &mut Ui, tokens: &Tokens, device: &fm_engine::devices::Device) {
    use fm_engine::devices::{Removal, display_label};
    let size =
        |bytes: Option<u64>| bytes.map_or_else(|| "Unavailable".into(), crate::listing::human_size);
    information(ui, tokens, "Volume", |ui| {
        card_rows(
            ui,
            tokens,
            "pv-volume",
            &[
                ("Name".into(), display_label(&device.name)),
                (
                    "Mounted at".into(),
                    display_label(&fm_engine::item_cmds::display_path(&device.mount)),
                ),
                ("Filesystem".into(), display_label(&device.filesystem)),
                ("Capacity".into(), size(device.total)),
                ("Free space".into(), size(device.free)),
            ],
        );
        if let (Some(free), Some(total)) = (device.free, device.total)
            && total > 0
        {
            let used = total.saturating_sub(free);
            ui.add(
                egui::ProgressBar::new((used as f64 / total as f64) as f32)
                    .text(format!("{} used", crate::listing::human_size(used))),
            );
        }
        if let Removal::Unsupported(reason) = &device.removal {
            crate::dialog_style::hint(ui, tokens, reason);
        }
    });
}

fn meta_card(ui: &mut Ui, tokens: &Tokens, m: &MetaDoc) {
    information(ui, tokens, "File information", |ui| {
        card_rows(ui, tokens, "pv-meta", &m.rows);
        if let Some(h) = &m.hint {
            crate::dialog_style::hint(ui, tokens, h);
        }
    });
}

fn audio_body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    model: &Arc<Preview>,
    meta: &MetaDoc,
    audio: &fm_engine::preview_audio_cmds::AudioState,
    actions: &mut Vec<Action>,
) {
    let output = paged_scroll(ui, ScrollArea::vertical(), ("pv-audio", pane))
        .auto_shrink(false)
        .show(ui, |ui| {
            crate::player_controls::player_row(
                ui,
                tokens,
                Id::NULL.with(("pv-audio-seek", pane)),
                &crate::player_controls::Player {
                    playing: audio.playing,
                    position: audio.position,
                    duration: audio.doc.duration.unwrap_or(0.0),
                    volume: audio.volume,
                    muted: audio.muted,
                    play: "preview.audio.play",
                    seek: "preview.audio.seek",
                    volume_command: "preview.audio.volume",
                    mute: "preview.audio.mute",
                },
                actions,
            );
            if let Some(error) = &audio.error {
                ui.colored_label(tokens.error, error);
            }
            ui.horizontal_wrapped(|ui| media_actions(ui, actions))
                .response
                .context_menu(|ui| {
                    if media_actions(ui, actions) {
                        ui.close();
                    }
                });
            if let Some(cover) = &meta.poster {
                image(ui, pane, tokens, model, cover, Zoom::Fit, None);
            }
            meta_card(ui, tokens, meta);
        });
    remember_scroll(ui, &output);
    if audio.playing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
}

fn media_actions(ui: &mut Ui, actions: &mut Vec<Action>) -> bool {
    let mut chosen = false;
    for (label, command, hint) in [
        (
            "Play in Media",
            "media.play",
            "Insert after the current Media item and play from the Preview position.",
        ),
        (
            "Play next",
            "media.play_next",
            "Insert after the current Media item without interrupting playback.",
        ),
        (
            "Add to queue",
            "media.enqueue",
            "Append to the end without interrupting playback.",
        ),
        (
            "Play folder",
            "media.play_folder",
            "Insert this folder's media after the current Media item and play.",
        ),
    ] {
        if ui.button(label).on_hover_text(hint).clicked() {
            actions.push((command, json!({})));
            chosen = true;
        }
    }
    chosen
}

/// A playable video: the newest frame (or the poster before one arrives), a play/seek bar
/// and the metadata rows below.
fn video_body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    p: &Arc<Preview>,
    m: &MetaDoc,
    v: &fm_engine::video_cmds::VideoState,
    actions: &mut Vec<Action>,
) {
    let ppp = ui.ctx().pixels_per_point();
    let avail = ui.available_size();
    let bucket = fm_engine::video_cmds::width_bucket(avail.x * ppp);
    if v.width != Some(bucket) {
        actions.push(("preview.video.fit", json!({ "width": avail.x * ppp })));
    }
    let output = paged_scroll(ui, ScrollArea::both(), ("pv-video", pane))
        .auto_shrink(false)
        .show(ui, |ui| {
            let (boxed, size) = video_box(ui, avail, &v.info);
            // The newest frame. A paused video that has not started shows its poster; one
            // that is starting does not, since the poster is taken ~10 s in and the first
            // frame (at 0:00) would replace it with a different picture a moment later.
            let picture = match &v.frame {
                Some(f) => Some(video_texture(
                    ui.ctx(),
                    Id::NULL.with(("pv-video-tex", pane)),
                    f,
                )),
                None if !v.playing => m
                    .poster
                    .as_ref()
                    .and_then(|poster| poster_texture(ui.ctx(), pane, p, poster)),
                None => None,
            };
            let resp = paint_video_box(ui, tokens, boxed, size, picture, Sense::click());
            if resp.clicked() {
                actions.push(("preview.video.play", json!({})));
            }
            let controls_top = ui.cursor().top();
            video_controls(ui, pane, tokens, v, actions);
            ui.horizontal_wrapped(|ui| media_actions(ui, actions))
                .response
                .context_menu(|ui| {
                    if media_actions(ui, actions) {
                        ui.close();
                    }
                });
            let used = ui.cursor().top() - controls_top;
            ui.data_mut(|d| d.insert_temp(controls_height_id(pane), used));
            if let Some(e) = &v.error {
                ui.colored_label(tokens.error, e);
            }
            if let Some(hint) = &v.hint {
                crate::dialog_style::hint(ui, tokens, hint);
            }
            ui.push_id(("pv-video-meta", pane), |ui| meta_card(ui, tokens, m));
        });
    remember_scroll(ui, &output);
    if v.playing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

/// Share of the pane height the video box takes; the controls always start right below it.
const VIDEO_BOX_SHARE: f32 = 0.75;

/// The video box (full width, three quarters of the pane) and the picture fitted and centred
/// in it. The box depends only on the pane size, never on the file, so switching videos or a
/// poster turning into frames never moves the controls. In a very short pane the box gives
/// up a fixed amount of room so the controls still fit.
fn video_box(ui: &Ui, avail: egui::Vec2, info: &fm_preview::VideoInfo) -> (egui::Vec2, egui::Vec2) {
    let height = video_box_height(ui, avail);
    let natural = vec2(info.width.max(1) as f32, info.height.max(1) as f32);
    let fit = (avail.x / natural.x).min(height / natural.y);
    // The Preview zoom (Ctrl+Plus/Minus) scales the picture; zoomed past the box, it grows.
    let picture = natural * fit * content_scale(ui);
    (vec2(avail.x.max(picture.x), height.max(picture.y)), picture)
}

fn video_box_height(ui: &Ui, avail: egui::Vec2) -> f32 {
    (avail.y * VIDEO_BOX_SHARE)
        .min(avail.y - controls_estimate(ui))
        .max(64.0)
}

/// A fixed allowance for the controls and media actions under the video box.
fn controls_estimate(ui: &Ui) -> f32 {
    4.0 * ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 6.0
}

/// Height of the controls and media actions as last drawn in `pane`, so the frame before a
/// new video's player exists (and the loading spinner) keeps the information card in place.
fn controls_height_id(pane: PaneId) -> Id {
    Id::NULL.with(("pv-video-controls-h", pane))
}

fn reserve_controls(ui: &mut Ui, pane: PaneId, width: f32) {
    let height = ui
        .data(|d| d.get_temp::<f32>(controls_height_id(pane)))
        .unwrap_or_else(|| controls_estimate(ui));
    // The measured height already holds the spacing that follows the last row.
    ui.allocate_space(vec2(width, (height - ui.spacing().item_spacing.y).max(0.0)));
}

/// Paint `texture` (or an empty box) fitted in the video box; the click response is returned.
fn paint_video_box(
    ui: &mut Ui,
    tokens: &Tokens,
    boxed: egui::Vec2,
    picture: egui::Vec2,
    texture: Option<egui::TextureHandle>,
    sense: Sense,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(boxed, sense);
    let at = egui::Rect::from_center_size(rect.center(), picture);
    match texture {
        Some(tex) => {
            egui::Image::new((tex.id(), picture)).paint_at(ui, at);
        }
        None => {
            ui.painter().rect_filled(at, 0.0, tokens.bg);
        }
    }
    response
}

/// A video whose player is not created yet (the frame between its preview arriving and the
/// engine's next tick, or a background refresh): the player's layout with the poster and the
/// controls' space left empty, so nothing moves when the player appears.
fn pending_video_body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    m: &MetaDoc,
    info: &fm_preview::VideoInfo,
) {
    let avail = ui.available_size();
    let output = paged_scroll(ui, ScrollArea::both(), ("pv-video", pane))
        .auto_shrink(false)
        .show(ui, |ui| {
            let (boxed, size) = video_box(ui, avail, info);
            // Empty: the player, a frame later, shows the poster only if it stays paused.
            paint_video_box(ui, tokens, boxed, size, None, Sense::hover());
            reserve_controls(ui, pane, avail.x);
            ui.push_id(("pv-video-meta", pane), |ui| meta_card(ui, tokens, m));
        });
    remember_scroll(ui, &output);
    ui.ctx().request_repaint();
}

fn is_video_entry(entry: &fm_vfs::Entry) -> bool {
    fm_preview::thumbs::classify(&entry.name) == fm_preview::thumbs::ThumbKind::Video
}

/// The poster's first frame as a texture, cached per pane and preview.
fn poster_texture(
    ctx: &egui::Context,
    pane: PaneId,
    p: &Arc<Preview>,
    poster: &ImageDoc,
) -> Option<egui::TextureHandle> {
    let (rgba, _) = poster.frames.first()?;
    let id = Id::NULL.with(("pv-video-poster", pane));
    let ident = Arc::as_ptr(p) as usize;
    if let Some((cached, texture)) = ctx.data(|d| d.get_temp::<(usize, egui::TextureHandle)>(id))
        && cached == ident
    {
        return Some(texture);
    }
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [poster.width as usize, poster.height as usize],
        rgba,
    );
    let texture = ctx.load_texture("video-poster", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, (ident, texture.clone())));
    Some(texture)
}

pub(crate) fn video_texture(
    ctx: &egui::Context,
    id: Id,
    frame: &Arc<fm_preview::player::Frame>,
) -> egui::TextureHandle {
    let cached =
        ctx.data(|d| d.get_temp::<(Arc<fm_preview::player::Frame>, egui::TextureHandle)>(id));
    let image = || {
        egui::ColorImage::from_rgba_unmultiplied(
            [frame.width as usize, frame.height as usize],
            &frame.rgba,
        )
    };
    let texture = match cached {
        Some((old, texture)) if Arc::ptr_eq(&old, frame) => texture,
        Some((_, mut texture)) => {
            texture.set(image(), egui::TextureOptions::LINEAR);
            texture
        }
        None => ctx.load_texture("video-frame", image(), egui::TextureOptions::LINEAR),
    };
    ctx.data_mut(|d| d.insert_temp(id, (frame.clone(), texture.clone())));
    texture
}

fn video_controls(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    v: &fm_engine::video_cmds::VideoState,
    actions: &mut Vec<Action>,
) {
    crate::player_controls::player_row(
        ui,
        tokens,
        Id::NULL.with(("pv-video-seek", pane)),
        &crate::player_controls::Player {
            playing: v.playing,
            position: v.pos,
            duration: v.info.duration.max(0.0),
            volume: v.volume,
            muted: v.muted,
            play: "preview.video.play",
            seek: "preview.video.seek",
            volume_command: "preview.video.volume",
            mute: "preview.video.mute",
        },
        actions,
    );
}

#[allow(clippy::too_many_arguments)]
fn thumb_toolbar(
    ui: &mut Ui,
    pane: PaneId,
    on: bool,
    size: usize,
    sort: ThumbSort,
    group: bool,
    recursive: bool,
    actions: &mut Vec<Action>,
) {
    let r = ui
        .add(egui::Button::selectable(recursive, "Recursive"))
        .on_hover_text("Include subfolders (also in Preferences)");
    if r.clicked() {
        actions.push(("prefs.preview.recursive", json!({ "value": !recursive })));
    }
    ui.separator();
    if ui.add(egui::Button::selectable(on, "Thumbnails")).clicked() {
        actions.push(("preview.thumbs.toggle", json!({})));
    }
    if on {
        for (ix, (label, name)) in [("S", "small"), ("M", "medium"), ("L", "large")]
            .into_iter()
            .enumerate()
        {
            let r = ui
                .add(egui::Button::selectable(size == ix, label))
                .on_hover_text(format!("{name} thumbnails"));
            if r.clicked() {
                actions.push(("preview.thumbs.size", json!({ "size": name })));
            }
        }
        ui.separator();
        let label_of = |choice: ThumbSort| match choice {
            ThumbSort::Name => "Name",
            ThumbSort::Modified => "Modified",
            ThumbSort::Type => "Type",
        };
        egui::ComboBox::from_id_salt(("pv-thumb-sort", pane))
            .selected_text(format!("Sort: {}", label_of(sort)))
            .show_ui(ui, |ui| {
                for (choice, name) in ThumbSort::ALL {
                    if ui
                        .selectable_label(sort == choice, label_of(choice))
                        .clicked()
                    {
                        actions.push(("preview.thumbs.sort", json!({ "by": name })));
                    }
                }
            })
            .response
            .on_hover_text("Order of the thumbnails");
        if recursive {
            let r = ui
                .add(egui::Button::selectable(group, "By folder"))
                .on_hover_text("Group files under their subfolder (off: one list of all files)");
            if r.clicked() {
                actions.push(("preview.thumbs.group", json!({ "on": !group })));
            }
        }
    }
}

/// Short type label for the tile of a file without a thumbnail.
fn tile_label(entry: &fm_vfs::Entry) -> String {
    if entry.is_dir_like() {
        return "Folder".into();
    }
    match entry.name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.chars().count() <= 5 => ext.to_uppercase(),
        _ => "File".into(),
    }
}

/// Virtualized thumbnail grid for a folder: only the rows on screen are laid out, and the
/// engine is told which window is visible so it decodes just those (plus a screenful around).
#[allow(clippy::too_many_arguments)]
const ARROW_KEYS: egui::EventFilter = egui::EventFilter {
    horizontal_arrows: true,
    vertical_arrows: true,
    tab: false,
    escape: false,
};

/// The thumbnail grid's selection, with the same click rules as the file listing:
/// click selects one, Ctrl toggles, Shift extends from the anchor.
#[derive(Clone, Default)]
struct ThumbSel {
    names: std::collections::BTreeSet<String>,
    anchor: usize,
}

impl ThumbSel {
    fn click(&mut self, items: &[fm_vfs::Entry], index: usize, shift: bool, toggle: bool) {
        let Some(entry) = items.get(index) else {
            return;
        };
        if shift {
            let (lo, hi) = (self.anchor.min(index), self.anchor.max(index));
            self.names = items
                .iter()
                .skip(lo)
                .take(hi - lo + 1)
                .map(|e| e.name.clone())
                .collect();
        } else if toggle {
            if !self.names.remove(&entry.name) {
                self.names.insert(entry.name.clone());
            }
            self.anchor = index;
        } else {
            self.names = std::iter::once(entry.name.clone()).collect();
            self.anchor = index;
        }
    }
}

#[derive(Clone, Copy)]
enum RowKind {
    Header(usize),
    Cells(usize, usize),
}

struct PlanRow {
    top: f32,
    h: f32,
    kind: RowKind,
}

/// Vertical layout of the grid: tile rows, plus a heading row per subfolder when grouped.
struct GridPlan {
    rows: Vec<PlanRow>,
    total: f32,
    /// Indices into `rows` of the tile rows, in order.
    cell_rows: Vec<usize>,
    /// For every item, its position in `cell_rows`.
    pos_of: Vec<usize>,
}

impl GridPlan {
    fn new(a: &fm_preview::arrange::Arranged, cols: usize, row_h: f32, head_h: f32) -> Self {
        let len = a.items.len();
        let mut plan = Self {
            rows: Vec::new(),
            total: 0.0,
            cell_rows: Vec::new(),
            pos_of: vec![0; len],
        };
        let spans: Vec<(Option<usize>, usize, usize)> = if a.groups.is_empty() {
            vec![(None, 0, len)]
        } else {
            a.groups
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    (
                        Some(i),
                        g.start,
                        a.groups.get(i + 1).map_or(len, |n| n.start),
                    )
                })
                .collect()
        };
        for (group, start, end) in spans {
            if let Some(group) = group {
                plan.push(head_h, RowKind::Header(group));
            }
            let mut at = start;
            while at < end {
                let next = (at + cols).min(end);
                plan.cell_rows.push(plan.rows.len());
                for slot in plan.pos_of.iter_mut().take(next).skip(at) {
                    *slot = plan.cell_rows.len() - 1;
                }
                plan.push(row_h, RowKind::Cells(at, next));
                at = next;
            }
        }
        plan
    }

    fn push(&mut self, h: f32, kind: RowKind) {
        self.rows.push(PlanRow {
            top: self.total,
            h,
            kind,
        });
        self.total += h;
    }

    fn cell_row(&self, index: usize) -> Option<&PlanRow> {
        self.rows
            .get(*self.cell_rows.get(*self.pos_of.get(index)?)?)
    }

    fn top_of(&self, index: usize) -> f32 {
        self.cell_row(index).map_or(0.0, |r| r.top)
    }

    /// Move `rows` tile rows up or down, keeping the column where the target row is long enough.
    fn step_rows(&self, from: usize, rows: isize) -> usize {
        let Some(RowKind::Cells(start, _)) = self.cell_row(from).map(|r| r.kind) else {
            return from;
        };
        let pos = self.pos_of.get(from).copied().unwrap_or(0) as isize;
        let last = self.cell_rows.len().saturating_sub(1) as isize;
        let want = pos + rows;
        // A page key past either end lands on the first or last tile.
        if rows.abs() > 1 && !(0..=last).contains(&want) {
            return if want < 0 {
                0
            } else {
                self.pos_of.len().saturating_sub(1)
            };
        }
        let to = want.clamp(0, last) as usize;
        let target = self
            .cell_rows
            .get(to)
            .and_then(|r| self.rows.get(*r))
            .map(|r| r.kind);
        match target {
            Some(RowKind::Cells(s, e)) => (s + (from - start)).min(e.saturating_sub(1)),
            _ => from,
        }
    }

    fn visible(&self, y0: f32, y1: f32) -> std::ops::Range<usize> {
        let first = self.rows.partition_point(|r| r.top + r.h <= y0);
        let end = self.rows.partition_point(|r| r.top < y1);
        first..end.max(first)
    }

    /// `(first, count)` of the tiles on the visible rows.
    fn window(&self, visible: &std::ops::Range<usize>) -> Option<(usize, usize)> {
        let mut cells = self
            .rows
            .get(visible.clone())?
            .iter()
            .filter_map(|r| match r.kind {
                RowKind::Cells(s, e) => Some((s, e)),
                RowKind::Header(_) => None,
            });
        let (first, mut last) = cells.next()?;
        for (_, e) in cells {
            last = e;
        }
        Some((first, last - first))
    }
}

/// The grid's display order, rebuilt only when the folder document, sort or grouping changes.
fn arranged_for(
    ctx: &egui::Context,
    pane: PaneId,
    d: &DirDoc,
    sort: ThumbSort,
    group: bool,
) -> Arc<fm_preview::arrange::Arranged> {
    let key = (
        d.items.as_ptr() as usize,
        d.items.len(),
        d.files,
        d.bytes,
        d.recursive,
        sort,
        group,
    );
    let id = Id::NULL.with(("pv-thumb-arranged", pane));
    type Cached = (
        (usize, usize, usize, u64, bool, ThumbSort, bool),
        Arc<fm_preview::arrange::Arranged>,
    );
    if let Some((k, a)) = ctx.data(|dt| dt.get_temp::<Cached>(id))
        && k == key
    {
        return a;
    }
    let a = Arc::new(fm_preview::arrange::arrange(d, sort, group));
    ctx.data_mut(|dt| dt.insert_temp(id, (key, a.clone())));
    a
}

#[allow(clippy::too_many_arguments)]
fn thumb_grid(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    d: &DirDoc,
    size: usize,
    sort: ThumbSort,
    group: bool,
    asked: Option<(usize, usize)>,
    actions: &mut Vec<Action>,
    open: &mut Option<fm_vfs::VfsUri>,
    dir: Option<fm_vfs::VfsUri>,
    published: &mut Option<fm_engine::file_cmds::GridSelection>,
) {
    use fm_engine::thumbs::ThumbState;
    use fm_preview::thumbs::EDGES;

    let arranged = arranged_for(ui.ctx(), pane, d, sort, group);
    let items = &arranged.items;
    if items.is_empty() {
        centered(ui, tokens, "(empty folder)");
        return;
    }
    let edge = EDGES.get(size).copied().unwrap_or(EDGES[0]);
    let gap = tokens.padding;
    let label_h = ui.fonts_mut(|f| f.row_height(&tokens.font())) + 2.0;
    let width = ui.available_width();
    let cell = (edge as f32 * content_scale(ui)).min((width - gap).max(32.0));
    let cols = (((width + gap) / (cell + gap)).floor() as usize).max(1);
    let row_h = cell + label_h + gap;
    let head_h = label_h + gap * 5.5;
    let plan = GridPlan::new(&arranged, cols, row_h, head_h);

    let ctx = ui.ctx().clone();
    let cache_id = Id::NULL.with(("pv-thumb-tex", pane));
    type Cache =
        std::collections::HashMap<usize, (Arc<fm_preview::thumbs::Thumb>, egui::TextureHandle)>;
    let mut cache: Cache = ctx.data_mut(|dt| dt.get_temp(cache_id)).unwrap_or_default();
    let mut used: Cache = Cache::new();
    let mut waiting = false;
    let mut window = None;

    // Keyboard cursor: only active once a cell was clicked (grid focus).
    let grid_id = Id::NULL.with(("pv-thumb-grid", pane));
    let state_id = Id::NULL.with(("pv-thumb-state", pane));
    let (mut cursor, mut offset, view_h): (usize, f32, f32) = ctx
        .data_mut(|dt| dt.get_temp(state_id))
        .unwrap_or((0, 0.0, 0.0));
    let count = items.len();
    // A grid that was not drawn last frame (just toggled on, or a new folder) takes the focus
    // with the cursor on the first item, so the keys work immediately.
    let pass = ctx.cumulative_pass_nr();
    let shown_id = Id::NULL.with(("pv-thumb-shown", pane));
    let want_id = Id::NULL.with(("pv-thumb-want-focus", pane));
    let last_shown: Option<u64> = ctx.data(|dt| dt.get_temp(shown_id));
    let sel_id = Id::NULL.with(("pv-thumb-sel", pane));
    let mut sel: ThumbSel = ctx.data_mut(|dt| dt.get_temp(sel_id)).unwrap_or_default();
    if last_shown.is_none_or(|n| pass.saturating_sub(n) > 2) {
        cursor = 0;
        offset = 0.0;
        sel = ThumbSel::default();
        ctx.data_mut(|dt| dt.insert_temp(want_id, true));
    }
    // Keep asking until egui reports the focus (the first pass of a window may be discarded).
    // A preview following another pane's cursor (e.g. landing on a folder) must not steal the
    // keyboard from that pane, so the grid only claims focus when its own pane is focused.
    let pane_focused = s.layout.focused() == pane;
    let want = ctx.data(|dt| dt.get_temp::<bool>(want_id)).unwrap_or(false);
    if pane_focused {
        if ctx.memory(|m| m.has_focus(grid_id)) {
            ctx.data_mut(|dt| dt.insert_temp(want_id, false));
        } else if want || ctx.memory(|m| m.focused().is_none()) {
            ctx.memory_mut(|m| m.request_focus(grid_id));
        }
    }
    ctx.data_mut(|dt| dt.insert_temp(shown_id, pass));
    cursor = cursor.min(count - 1);
    let mut scroll_to = None;
    // Registering the widget keeps egui from dropping focus as "disappeared".
    ui.interact(ui.max_rect(), grid_id, Sense::focusable_noninteractive());
    let active = ctx.memory_mut(|m| {
        let mut on = m.has_focus(grid_id);
        if on && !pane_focused {
            m.surrender_focus(grid_id);
            on = false;
        }
        if on {
            m.set_focus_lock_filter(grid_id, ARROW_KEYS);
        }
        on
    });
    ctx.data_mut(|dt| dt.insert_temp(Id::NULL.with("pv-grid-active"), active));
    if active {
        let page_rows = ((view_h / row_h).floor() as usize).saturating_sub(1).max(1);
        let step = |from: usize, rows: isize| plan.step_rows(from, rows);
        let mut target = cursor;
        ui.input_mut(|i| {
            use egui::{Key, Modifiers};
            if i.consume_key(Modifiers::COMMAND, Key::A) {
                sel.names = items.iter().map(|e| e.name.clone()).collect();
            }
            if i.consume_key(Modifiers::NONE, Key::Enter)
                && let Some(entry) = items.get(cursor)
            {
                *open = Some(entry.uri.clone());
            }
            // Shift variants extend the selection; the movement itself is the same.
            let mut key =
                |k| i.consume_key(Modifiers::SHIFT, k) | i.consume_key(Modifiers::NONE, k);
            if key(Key::ArrowLeft) {
                target = target.saturating_sub(1);
            }
            if key(Key::ArrowRight) {
                target = (target + 1).min(count - 1);
            }
            if key(Key::ArrowUp) {
                target = step(target, -1);
            }
            if key(Key::ArrowDown) {
                target = step(target, 1);
            }
            if key(Key::PageUp) {
                target = step(target, -(page_rows as isize));
            }
            if key(Key::PageDown) {
                target = step(target, page_rows as isize);
            }
            if key(Key::Home) {
                target = 0;
            }
            if key(Key::End) {
                target = count - 1;
            }
        });
        if target != cursor {
            cursor = target;
            sel.click(items, cursor, ui.input(|i| i.modifiers.shift), false);
            // Minimal scroll: only move when the cursor row leaves the viewport.
            let top = plan.top_of(cursor);
            let bottom = top + row_h - gap;
            if top < offset {
                scroll_to = Some(top);
            } else if bottom > offset + view_h {
                scroll_to = Some(bottom - view_h);
            }
        }
    }

    let mut area = ScrollArea::vertical()
        .id_salt(("pv-thumbs", pane))
        .auto_shrink(false);
    if let Some(y) = scroll_to {
        area = area.vertical_scroll_offset(y.max(0.0));
    }
    let out = area.show_viewport(ui, |ui, viewport| {
        let origin = ui.cursor().min;
        ui.allocate_space(vec2(width, plan.total));
        let visible = plan.visible(viewport.min.y, viewport.max.y);
        window = plan.window(&visible);
        for row_ix in visible {
            let Some(row) = plan.rows.get(row_ix) else {
                break;
            };
            let (first, last) = match row.kind {
                RowKind::Header(group) => {
                    if let Some(g) = arranged.groups.get(group) {
                        let end = arranged
                            .groups
                            .get(group + 1)
                            .map_or(items.len(), |next| next.start);
                        let title = if g.folder.is_empty() {
                            "This folder".to_string()
                        } else {
                            g.folder.clone()
                        };
                        // A filled band with an accent edge, after some air above, so a
                        // new folder reads as a section break rather than another caption.
                        let band = egui::Rect::from_min_size(
                            origin + vec2(0.0, row.top + gap * 3.5),
                            vec2(width, head_h - gap * 4.0),
                        );
                        let painter = ui.painter();
                        painter.rect_filled(band, tokens.corner_radius, tokens.hover_bg);
                        painter.rect_filled(
                            egui::Rect::from_min_size(band.min, vec2(4.0, band.height())),
                            tokens.corner_radius,
                            tokens.accent,
                        );
                        let title_pos = band.left_center() + vec2(gap * 2.0, 0.0);
                        let title_rect = painter.text(
                            title_pos,
                            egui::Align2::LEFT_CENTER,
                            title,
                            tokens.font(),
                            tokens.text,
                        );
                        painter.text(
                            title_rect.right_center() + vec2(gap, 0.0),
                            egui::Align2::LEFT_CENTER,
                            format!("{} files", end - g.start),
                            tokens.font(),
                            tokens.text_muted,
                        );
                    }
                    continue;
                }
                RowKind::Cells(first, last) => (first, last),
            };
            for index in first..last {
                let Some(entry) = items.get(index) else {
                    break;
                };
                let col = index - first;
                let rect = egui::Rect::from_min_size(
                    origin + vec2(col as f32 * (cell + gap), row.top),
                    vec2(cell, row_h - gap),
                );
                let resp = ui.interact(
                    rect,
                    Id::NULL.with(("pv-thumb-cell", pane, index)),
                    Sense::click_and_drag(),
                );
                if resp.drag_started() {
                    if !sel.names.contains(&entry.name) {
                        cursor = index;
                        sel.click(items, index, false, false);
                    }
                    let sources: Vec<_> = items
                        .iter()
                        .filter(|e| sel.names.contains(&e.name))
                        .map(|e| e.uri.clone())
                        .collect();
                    resp.dnd_set_drag_payload(crate::dnd::DragPayload {
                        sources,
                        origin: pane,
                    });
                }
                if resp.clicked() || resp.secondary_clicked() {
                    cursor = index;
                    let m = ui.input(|i| i.modifiers);
                    if resp.clicked() {
                        sel.click(items, index, m.shift, m.command);
                    } else if !sel.names.contains(&entry.name) {
                        sel.click(items, index, false, false);
                    }
                    ctx.memory_mut(|m| {
                        m.request_focus(grid_id);
                        m.set_focus_lock_filter(grid_id, ARROW_KEYS);
                    });
                }
                if resp.double_clicked() {
                    *open = Some(entry.uri.clone());
                }
                crate::context_menu::attach(&resp, s, &entry.uri, open);
                let focused = ctx.memory(|m| m.has_focus(grid_id));
                if sel.names.contains(&entry.name) {
                    ui.painter()
                        .rect_filled(rect, tokens.corner_radius, tokens.selection_bg);
                } else if resp.hovered() {
                    ui.painter()
                        .rect_filled(rect, tokens.corner_radius, tokens.hover_bg);
                }
                if focused && index == cursor {
                    ui.painter().rect_stroke(
                        rect,
                        tokens.corner_radius,
                        egui::Stroke::new(tokens.border_width.max(1.5), tokens.accent),
                        egui::StrokeKind::Inside,
                    );
                }
                let art = egui::Rect::from_min_size(rect.min, vec2(cell, cell));
                match s.thumbs.get(entry, edge) {
                    ThumbState::Ready(t) => {
                        let key = Arc::as_ptr(&t) as usize;
                        let (_, tex) = cache.remove(&key).unwrap_or_else(|| {
                            let ci = egui::ColorImage::from_rgba_unmultiplied(
                                [t.width as usize, t.height as usize],
                                &t.rgba,
                            );
                            let tex = ctx.load_texture(
                                format!("pv-thumb-{pane}-{key}"),
                                ci,
                                egui::TextureOptions::LINEAR,
                            );
                            (t.clone(), tex)
                        });
                        let natural = vec2(t.width as f32, t.height as f32);
                        let fit = cell / edge as f32;
                        let draw = natural * fit;
                        let at = egui::Rect::from_center_size(art.center(), draw);
                        egui::Image::new((tex.id(), draw)).paint_at(ui, at);
                        used.insert(key, (t, tex));
                    }
                    state => {
                        waiting |= matches!(state, ThumbState::Pending);
                        let inner = art.shrink(gap);
                        ui.painter().rect_stroke(
                            inner,
                            tokens.corner_radius,
                            egui::Stroke::new(tokens.border_width, tokens.border),
                            egui::StrokeKind::Inside,
                        );
                        let text = if matches!(state, ThumbState::Pending) {
                            "…".to_string()
                        } else {
                            tile_label(entry)
                        };
                        ui.painter().text(
                            inner.center(),
                            egui::Align2::CENTER_CENTER,
                            text,
                            tokens.font(),
                            tokens.text_muted,
                        );
                    }
                }
                let mut job = LayoutJob::simple(
                    fm_preview::arrange::base_name(&entry.name).to_string(),
                    tokens.font(),
                    tokens.text,
                    cell,
                );
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                let pos = egui::pos2(
                    rect.center().x - galley.size().x / 2.0,
                    rect.min.y + cell + 1.0,
                );
                ui.painter().galley(pos, galley, tokens.text);
                resp.on_hover_text(&entry.name);
            }
        }
    });
    offset = out.state.clamped_offset().y;
    ctx.data_mut(|dt| dt.insert_temp(state_id, (cursor, offset, out.inner_rect.height())));
    if let Some(dir) = dir {
        *published = Some(fm_engine::file_cmds::GridSelection {
            pane,
            dir,
            items: items
                .iter()
                .filter(|e| sel.names.contains(&e.name))
                .map(|e| e.uri.clone())
                .collect(),
        });
    }
    ctx.data_mut(|dt| dt.insert_temp(sel_id, sel));
    ctx.data_mut(|dt| dt.insert_temp(cache_id, used));
    if waiting {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    if let Some(w) = window
        && asked != Some(w)
    {
        actions.push((
            "preview.thumbs.visible",
            json!({ "first": w.0, "count": w.1 }),
        ));
    }
    if d.items_truncated {
        ui.label(tokens.muted(format!("Showing the first {} entries", items.len())));
    }
}

const SUBDIRS_SHOWN: usize = 12;

fn dir_card(ui: &mut Ui, tokens: &Tokens, d: &DirDoc) {
    information(ui, tokens, "Folder summary", |ui| {
        dir_details(ui, tokens, d)
    });
}

fn dir_details(ui: &mut Ui, tokens: &Tokens, d: &DirDoc) {
    let mut rows = vec![
        ("Files".to_string(), d.files.to_string()),
        ("Folders".to_string(), d.dirs.to_string()),
        ("Other".to_string(), d.other.to_string()),
        (
            "Total size".to_string(),
            format!(
                "{} ({} bytes)",
                crate::listing::human_size(d.bytes),
                d.bytes
            ),
        ),
    ];
    if d.files > 0 {
        rows.push((
            "Average file".to_string(),
            crate::listing::human_size(d.bytes / d.files as u64),
        ));
    }
    if let Some((name, size)) = &d.largest {
        rows.push((
            "Largest".to_string(),
            format!("{name} ({})", crate::listing::human_size(*size)),
        ));
    }
    if d.hidden > 0 {
        rows.push(("Hidden".to_string(), d.hidden.to_string()));
    }
    if let Some((name, _)) = &d.newest {
        rows.push(("Newest".to_string(), name.clone()));
    }
    if let Some((name, _)) = &d.oldest {
        rows.push(("Oldest".to_string(), name.clone()));
    }
    if d.recursive {
        rows.push(("Scope".to_string(), "all subfolders".to_string()));
    }
    card_rows(ui, tokens, "pv-dir", &rows);
    let mut subdirs: Vec<(&String, &(usize, u64))> = d.subdirs.iter().collect();
    subdirs.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(b.0)));
    let more = subdirs.len().saturating_sub(SUBDIRS_SHOWN);
    if !subdirs.is_empty() {
        ui.add_space(tokens.padding);
        crate::dialog_style::section_heading(ui, tokens, "Subfolders");
        let width = ui.available_width();
        let size_width = tokens.font_size * 5.5;
        let count_width = tokens.font_size * 3.0;
        let name_width = (width - size_width - count_width - 24.0).max(20.0);
        egui::Grid::new("pv-subdirs")
            .num_columns(3)
            .min_row_height(tokens.font_size + 4.0)
            .spacing([12.0, 5.0])
            .show(ui, |ui| {
                for (title, width) in [
                    ("Name", name_width),
                    ("Size", size_width),
                    ("Files", count_width),
                ] {
                    ui.allocate_ui_with_layout(
                        vec2(width, tokens.font_size + 4.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.label(RichText::new(title).color(tokens.text_muted));
                        },
                    );
                }
                ui.end_row();
                for (name, (count, bytes)) in subdirs.into_iter().take(SUBDIRS_SHOWN) {
                    ui.allocate_ui_with_layout(
                        vec2(name_width, tokens.font_size + 4.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.add(egui::Label::new(format!("{name}/")).truncate())
                                .on_hover_text(name);
                        },
                    );
                    ui.label(crate::listing::human_size(*bytes));
                    ui.label(count.to_string());
                    ui.end_row();
                }
            });
        if more > 0 {
            crate::dialog_style::hint(ui, tokens, format!("and {more} more folders"));
        }
    }
    if d.unreadable > 0 {
        ui.label(
            RichText::new(format!("{} folder(s) could not be read", d.unreadable))
                .color(tokens.warning),
        );
    }
    if d.scanning {
        ui.horizontal(|ui| {
            ui.spinner();
            crate::dialog_style::hint(ui, tokens, "Still scanning; totals so far");
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }
    if d.truncated {
        let why = if d.recursive {
            "The scan stopped early; totals only cover the folders listed before that"
        } else {
            "Only the first entries were counted"
        };
        crate::dialog_style::hint(ui, tokens, why);
    }
}

// ---- log mode ----

fn log_toolbar(ui: &mut Ui, pane: PaneId, v: &LogView, actions: &mut Vec<Action>) {
    let id = Id::NULL.with(("pv-logui", pane));
    let mut st: LogUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    st.query.clone_from(&v.filter.query);
    st.regex = v.filter.regex;
    let before = st.clone();

    if !v.sources.is_empty() {
        let response = egui::ComboBox::from_id_salt(("pv-log-order", pane))
            .selected_text(if v.chronological {
                "Chronological"
            } else {
                "Concatenate"
            })
            .show_ui(ui, |ui| {
                for (label, mode, on) in [
                    ("Concatenate", "concatenate", !v.chronological),
                    ("Chronological", "chronological", v.chronological),
                ] {
                    if ui.selectable_label(on, label).clicked() {
                        actions.push(("preview.log.order", json!({ "mode": mode })));
                    }
                }
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, response.enabled(), "Preview order")
        });
        response.on_hover_text("Concatenate in file-pane order, or merge by timestamp (unknown times last). Filename gutter is not copied.");
    }
    let current = v.filter.min_level.map_or("All", Level::label);
    let severity = egui::ComboBox::from_id_salt(("pv-level", pane))
        .selected_text(current)
        .width(70.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(v.filter.min_level.is_none(), "All")
                .clicked()
            {
                actions.push(("preview.log.level", json!({ "level": null })));
            }
            for l in Level::ALL {
                if ui
                    .selectable_label(v.filter.min_level == Some(l), l.label())
                    .clicked()
                {
                    actions.push((
                        "preview.log.level",
                        json!({ "level": l.label().to_lowercase() }),
                    ));
                }
            }
        })
        .response;
    severity.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ComboBox, severity.enabled(), "Minimum severity")
    });
    severity.on_hover_ui(|ui| {
        log_filter_help(ui, "Minimum severity", |ui| {
            ui.label("Show the selected level and anything more severe.");
            ui.label(RichText::new("Warning shows Warning, Error and Critical.").strong());
            ui.add_space(4.0);
            ui.label("All shows every severity.");
            ui.label("Unrecognised levels count as Info.");
            ui.add_space(4.0);
            ui.label("Low to high:");
            ui.label("Debug < Info < Notice < Warning < Error < Critical");
        });
    });
    let query_response = ui.add(
        egui::TextEdit::singleline(&mut st.query)
            .id(Id::NULL.with(("pv-log-query", pane)))
            .hint_text("Search")
            .desired_width(120.0),
    )
    .on_hover_text(
        "Search loaded records across the selected files. Enable .* to use a regular expression.",
    );
    query_response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Search log records")
    });
    if ui
        .data_mut(|d| d.remove_temp::<bool>(Id::NULL.with(("pv-log-request-focus", pane))))
        .unwrap_or(false)
    {
        query_response.request_focus();
    }
    ui.checkbox(&mut st.regex, ".*").on_hover_ui(|ui| {
        log_filter_help(ui, "Regular-expression search", |ui| {
            ui.label("On: search using patterns.");
            ui.label("Off: search for literal text.");
            ui.add_space(4.0);
            ui.label(RichText::new("Example: error|warning").strong());
            ui.label("Matches either \"error\" or \"warning\".");
        });
    });
    if st.query != before.query || st.regex != before.regex {
        actions.push((
            "preview.log.search",
            json!({ "query": st.query, "regex": st.regex }),
        ));
    }
    if ui
        .button("Time")
        .on_hover_text("Choose a quick range or custom From and To dates and times (UTC).")
        .clicked()
    {
        st.time = Some(time::TimePicker::new(v));
    }

    let (can_follow, access, more) = match v.doc.try_read() {
        Ok(d) => (
            v.sources.is_empty() && d.can_follow(),
            d.format.is_access(),
            d.more_before,
        ),
        Err(_) => (false, false, false),
    };
    if !v.sources.is_empty() {
        if ui
            .add_enabled(!v.is_busy(), egui::Button::new("Refresh"))
            .on_hover_text("Reload the selected files with their latest changes, keeping filters, ordering and display settings.")
            .on_disabled_hover_text("Wait for the current preview operation to finish.")
            .clicked()
        {
            actions.push(("preview.reload", json!({})));
        }
    } else if ui
        .add_enabled(can_follow, egui::Button::selectable(v.follow, "Live tail"))
        .on_hover_ui(|ui| {
            log_filter_help(ui, "Live tail", |ui| {
                ui.label("Checks file size on a background worker, including over SFTP and SMB.");
                ui.label("Unchanged: no file content downloaded. Growth: reads only appended bytes from the last offset.");
                ui.add_space(4.0);
                ui.label("Opening loads a small header and the configured tail window, not the whole file by default.");
                ui.label("Markers survive normal appends. If the file shrinks, its tail reloads and markers reset. Same-size replacements are not detected.");
                ui.add_space(4.0);
                ui.label("Set the polling interval and initial tail size in Preferences > Preview.");
                ui.label("Scrolling up pauses live tail.");
            });
        })
        .on_disabled_hover_text("This log cannot be followed")
        .clicked()
    {
        actions.push(("preview.log.follow", json!({})));
    }
    if access
        && ui
            .selectable_label(v.columns, "Columns")
            .on_hover_text("Show access-log fields in columns instead of raw record text.")
            .clicked()
    {
        actions.push(("preview.log.columns", json!({})));
    }
    if ui
        .selectable_label(v.expand_all, "Expand")
        .on_hover_ui(|ui| {
            log_filter_help(ui, "Expand log records", |ui| {
                ui.label("Show every line of multiline records, including stack traces.");
                ui.add_space(4.0);
                ui.label("Turn off to collapse records to their first line.");
            });
        })
        .clicked()
    {
        actions.push((
            "preview.log.fold",
            json!({ "mode": if v.expand_all { "collapse" } else { "expand" } }),
        ));
    }
    if ui
        .button("Mark")
        .on_hover_text("Set a marker below the last record (Ctrl+Shift+M)")
        .clicked()
    {
        actions.push(("preview.log.mark", json!({})));
    }
    if more
        && ui
            .add_enabled(
                v.sources.is_empty() || v.window_bytes < fm_engine::preview_multi::MAX_WINDOW,
                egui::Button::new("Load earlier"),
            )
            .on_hover_text("Load an older bounded chunk of records from the selected files.")
            .on_disabled_hover_text("The maximum preview window is already loaded.")
            .clicked()
    {
        actions.push(("preview.log.earlier", json!({})));
    }
    let clear_help = "Clear all filters: reset severity, search, regex and time bounds. Ordering and display options stay unchanged.";
    if ui
        .add_enabled(!v.filter.is_empty(), egui::Button::new("Clear"))
        .on_hover_text(clear_help)
        .on_disabled_hover_text("No active filters to clear.")
        .clicked()
    {
        st.query.clear();
        st.regex = false;
        st.time = None;
        actions.push(("preview.log.clear_filters", json!({})));
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

fn log_filter_help(ui: &mut Ui, title: &str, body: impl FnOnce(&mut Ui)) {
    ui.set_max_width(360.0);
    let size = egui::TextStyle::Body.resolve(ui.style()).size;
    ui.style_mut().override_font_id = Some(egui::FontId::proportional(size));
    ui.vertical(|ui| {
        ui.label(RichText::new(title).strong());
        ui.separator();
        body(ui);
        ui.add_space(4.0);
        ui.label("Only the preview changes, not the file.");
    });
}

fn log_body(ui: &mut Ui, pane: PaneId, tokens: &Tokens, v: &LogView, actions: &mut Vec<Action>) {
    let Ok(doc) = v.doc.try_read() else {
        centered(ui, tokens, "Updating…");
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        return;
    };
    let total = doc.records.len();
    let shown = v.filtered.as_ref().map_or(total, |f| f.len());
    let mut status = format!("{} · {shown} of {total} records", doc.format.label());
    if doc.gz {
        status.push_str(" · gzip");
    }
    if let Some(n) = &doc.note {
        status.push_str(&format!(" · {n}"));
    }
    ui.label(tokens.muted(status));
    if v.sources.iter().any(|source| source.note.is_some()) {
        ui.collapsing("Source details", |ui| {
            for source in &v.sources {
                if let Some(note) = &source.note {
                    ui.label(tokens.muted(format!("{}: {note}", source.entry.name)));
                }
            }
        });
        for source in &v.sources {
            if let Some(note) = &source.note
                && note.starts_with("Cannot preview:")
            {
                ui.label(
                    RichText::new(format!("{}: {note}", source.entry.name)).color(tokens.error),
                );
            }
        }
    }
    if let Some(e) = &v.filter_error {
        ui.label(RichText::new(e).color(tokens.error));
    }

    let columns = v.columns && doc.format.is_access();
    let folding = v.expand_all || !v.expanded.is_empty();
    let record_at = |i: usize| -> Option<usize> {
        match &v.filtered {
            Some(f) => f.get(i).map(|x| *x as usize),
            None => Some(i),
        }
    };
    let rows: Option<Vec<(usize, usize)>> = folding.then(|| {
        let mut end = 0usize;
        (0..shown)
            .filter_map(record_at)
            .map(|r| {
                let lines = doc.records.get(r).map_or(1, |rec| {
                    if v.expand_all || v.expanded.contains(&v.record_key(r, rec)) {
                        rec.text.lines().count().max(1)
                    } else {
                        1
                    }
                });
                end = end.saturating_add(lines);
                (r, end)
            })
            .collect()
    });
    let body_rows = rows
        .as_ref()
        .map_or(shown, |rows| rows.last().map_or(0, |(_, end)| *end));
    let end_mark = doc.marks.contains(&(doc.base + total as i64));
    let count = body_rows + usize::from(end_mark);
    let row_h = line_height(tokens);
    tight_rows(ui);

    let area = ScrollArea::both().auto_shrink(false).stick_to_bottom(true);
    let out = paged_scroll(ui, area, ("pv-log", pane)).show_rows(ui, row_h, count, |ui, range| {
        for i in range {
            if i >= body_rows {
                marker_row(ui, tokens, &doc, doc.marks.len(), row_h);
                continue;
            }
            let (r, line) = match &rows {
                Some(rows) => {
                    let index = rows.partition_point(|(_, end)| *end <= i);
                    let Some((r, _)) = rows.get(index) else {
                        continue;
                    };
                    let start = index
                        .checked_sub(1)
                        .and_then(|index| rows.get(index))
                        .map_or(0, |(_, end)| *end);
                    (*r, i - start)
                }
                None => match record_at(i) {
                    Some(r) => (r, 0),
                    None => continue,
                },
            };
            let Some(rec) = doc.records.get(r) else {
                continue;
            };
            if line == 0
                && let Some(n) = doc.marks.iter().position(|m| *m == doc.base + r as i64)
            {
                marker_rule(ui, tokens, n + 1, row_h);
            }
            let source = v.record_source(r);
            log_row(
                ui,
                tokens,
                &doc,
                v,
                rec,
                v.record_key(r, rec),
                source.map(|s| &s.entry),
                line,
                columns,
                row_h,
                actions,
            );
        }
    });
    remember_scroll(ui, &out);
    if v.follow
        && (ui
            .data(|d| d.get_temp::<bool>(Id::NULL.with("pv-page-used")))
            .unwrap_or(false)
            || (out
                .inner_rect
                .contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default())
                && ui.input(|i| i.smooth_scroll_delta.y > 0.0)))
    {
        actions.push(("preview.log.follow", json!({ "on": false })));
    }
    if v.follow {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// A rule with a label, drawn over the top edge of the next row.
fn marker_rule(ui: &mut Ui, tokens: &Tokens, n: usize, row_h: f32) {
    let rect = egui::Rect::from_min_size(
        ui.cursor().min,
        vec2(ui.available_width().max(900.0), row_h),
    );
    let p = ui.painter_at(rect);
    p.hline(
        rect.x_range(),
        rect.top() + 1.0,
        egui::Stroke::new(2.0, tokens.accent),
    );
    p.text(
        rect.left_top() + vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!("marker {n}"),
        egui::FontId::proportional(9.0),
        tokens.accent_text,
    );
}

/// A marker with no record after it yet.
fn marker_row(ui: &mut Ui, tokens: &Tokens, doc: &LogDoc, n: usize, row_h: f32) {
    let _ = doc;
    marker_rule(ui, tokens, n, row_h);
    ui.allocate_exact_size(vec2(ui.available_width().max(900.0), row_h), Sense::hover());
}

#[allow(clippy::too_many_arguments)]
fn log_row(
    ui: &mut Ui,
    tokens: &Tokens,
    doc: &LogDoc,
    v: &LogView,
    rec: &LogRecord,
    record_key: u64,
    source: Option<&fm_vfs::Entry>,
    line: usize,
    columns: bool,
    row_h: f32,
    actions: &mut Vec<Action>,
) {
    let font = mono(tokens);
    let color = if rec.orphan {
        tokens.text_muted
    } else {
        tokens.level(rec.level)
    };
    let folded = !v.expand_all && !v.expanded.contains(&record_key);
    let extra = rec.extra_lines();

    let text = rec.text.lines().nth(line).unwrap_or("");
    ui.horizontal(|ui| {
        tight_rows(ui);
        if let Some(source) = source {
            ui.add_sized(
                vec2(200.0, row_h),
                egui::Label::new(
                    RichText::new(&source.name)
                        .font(font.clone())
                        .color(tokens.text_muted)
                        .line_height(Some(row_h)),
                )
                .truncate()
                .halign(egui::Align::LEFT)
                .selectable(false),
            )
            .on_hover_text(source.uri.to_string());
        }
        if line == 0 && extra > 0 {
            if ui
                .add_sized(
                    vec2(18.0, row_h),
                    egui::Button::new(if folded { ">" } else { "v" }).frame(false),
                )
                .clicked()
            {
                actions.push(("preview.log.fold", json!({ "record": record_key })));
            }
        } else {
            ui.add_space(18.0);
        }
        if columns && doc.format.is_access() && rec.fields.len() >= 6 {
            for (i, field) in rec.fields.iter().take(6).enumerate() {
                ui.add_sized(
                    vec2(if i == 3 { 300.0 } else { 110.0 }, row_h),
                    egui::Label::new(
                        RichText::new(field)
                            .font(font.clone())
                            .color(color)
                            .line_height(Some(row_h)),
                    )
                    .truncate()
                    .selectable(true),
                );
            }
        } else {
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .font(font.clone())
                        .line_height(Some(row_h))
                        .color(if line > 0 { tokens.text_muted } else { color }),
                )
                .extend()
                .selectable(true),
            );
        }
        if line == 0 && folded && extra > 0 {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("+{extra} lines"))
                        .font(font)
                        .color(tokens.text_muted)
                        .line_height(Some(row_h)),
                )
                .selectable(false),
            );
        }
    });
}

/// True while a thumbnail grid owns the keyboard, so the file listing must not also act on keys.
pub fn grid_has_keyboard(ctx: &egui::Context) -> bool {
    ctx.data(|dt| dt.get_temp::<bool>(Id::NULL.with("pv-grid-active")))
        .unwrap_or(false)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use fm_layout::PaneKind;

    #[test]
    fn preview_add_button_targets_its_own_media_without_interrupting_it() {
        use egui_kittest::{Harness, kittest::Queryable};
        for video in [false, true] {
            let mut s = Session::new().expect("session");
            s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
                .expect("preview");
            let pane = s.layout.focused();
            s.poll();
            s.previews.slots.get_mut(&pane).expect("slot").body =
                Body::Static(Arc::new(Preview::Meta(MetaDoc {
                    rows: Vec::new(),
                    hint: None,
                    poster: None,
                    video: video.then(|| fm_preview::VideoInfo {
                        path: "/music/preview.mkv".into(),
                        range: None,
                        width: 320,
                        height: 180,
                        fps: 25.0,
                        duration: 120.0,
                        audio: None,
                        audio_delay: 0.0,
                        tracks: Default::default(),
                    }),
                    audio: (!video).then(|| fm_preview::audio::AudioDoc {
                        path: "/music/preview.mp3".into(),
                        duration: Some(120.0),
                    }),
                })));
            s.poll();
            let slot = s.previews.slots.get_mut(&pane).expect("slot");
            if let Some(video) = &mut slot.video {
                video.pos = 42.0;
                video.playing = true;
            } else {
                let audio = slot.audio.as_mut().expect("audio");
                audio.position = 42.0;
                audio.playing = true;
            }
            let mut harness = Harness::builder()
                .with_size(egui::vec2(640.0, 360.0))
                .build_ui_state(
                    move |ui, s| {
                        show(ui, s, pane, &Tokens::dark());
                    },
                    s,
                );
            harness.run();
            harness.get_by_label("Add to queue").click();
            harness.run();
            assert_eq!(harness.state().media.queue.items().len(), 1);
            assert_eq!(
                harness.state().media.queue.items()[0].file_name(),
                Some(if video { "preview.mkv" } else { "preview.mp3" })
            );
            let slot = &harness.state().previews.slots[&pane];
            if let Some(video) = &slot.video {
                assert!(video.playing);
                assert_eq!(video.pos, 42.0);
            } else {
                let audio = slot.audio.as_ref().expect("audio");
                assert!(audio.playing);
                assert_eq!(audio.position, 42.0);
            }
        }
    }

    #[test]
    fn subject_title_identifies_files_folders_and_remote_paths() {
        use fm_vfs::{Entry, EntryKind};
        for (uri, kind, expected) in [
            ("/home/alice/notes.txt", EntryKind::File, "/home/alice"),
            ("/home/alice/photos", EntryKind::Dir, "/home/alice/photos"),
            ("/", EntryKind::Dir, "/"),
            (
                "sftp://alice@server/home/alice/notes.txt",
                EntryKind::File,
                "sftp://alice@server/home/alice",
            ),
            (
                "smb://server/share/photos",
                EntryKind::Dir,
                "smb://server/share/photos",
            ),
        ] {
            let mut entry = Entry::new(VfsUri::parse(uri).expect("uri"), kind);
            let expected_uri = VfsUri::parse(expected).expect("uri");
            let expected = expected_uri
                .local_path_string()
                .unwrap_or_else(|| expected_uri.to_string());
            assert_eq!(subject_title(&entry).1.as_deref(), Some(expected.as_str()));
            entry.name = ".".into();
            assert!(subject_title(&entry).0.is_empty());
        }
        let mut link = Entry::new(
            VfsUri::parse("/home/alice/photos").expect("uri"),
            EntryKind::Symlink {
                target: "/pictures".into(),
            },
        );
        link.link_to_dir = true;
        assert_eq!(subject_title(&link).1, link.uri.local_path_string());
    }

    #[test]
    fn middle_ellipsis_preserves_both_ends_and_respects_the_width() {
        let measure = |text: &str| text.chars().count() as f32;
        assert_eq!(middle_ellipsis("/home/alice", 11.0, measure), "/home/alice");
        assert_eq!(middle_ellipsis("/home/alice", 7.0, measure), "/ho…ice");
        assert_eq!(middle_ellipsis("/写真/休暇/画像", 5.0, measure), "/写…画像");
        assert_eq!(middle_ellipsis("abc", 1.0, measure), "…");
        assert_eq!(middle_ellipsis("abc", 0.0, measure), "");
        assert_eq!(middle_ellipsis("", 0.0, measure), "");
    }

    #[test]
    fn toolbar_title_truncates_name_before_middle_of_folder_on_resize() {
        fn texts(shape: &egui::Shape, output: &mut Vec<Arc<egui::Galley>>) {
            match shape {
                egui::Shape::Text(text) => output.push(text.galley.clone()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        texts(shape, output);
                    }
                }
                _ => {}
            }
        }
        let name = "a-very-long-preview-filename-with-many-characters.txt";
        let folder = "/home/alice/Pictures/Screenshots";
        for tokens in [Tokens::dark(), Tokens::light()] {
            let ctx = egui::Context::default();
            crate::theme::apply_to(&ctx, &tokens);
            for width in [1200.0, 320.0, 100.0, 8.0, 1200.0] {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.horizontal(|ui| toolbar_title(ui, name, Some(folder), width));
                    });
                });
                let mut labels = Vec::new();
                for shape in &out.shapes {
                    texts(&shape.shape, &mut labels);
                }
                if width >= 320.0 {
                    assert!(labels.iter().any(|g| g.text() == folder && !g.elided));
                    let name_galley = labels.iter().find(|g| g.text() == name).expect("name");
                    assert_eq!(name_galley.elided, width == 320.0);
                } else if width == 100.0 {
                    let path = labels
                        .iter()
                        .find(|g| g.text().contains('/'))
                        .expect("path");
                    assert!(path.text().contains('…'));
                    assert!(path.text().starts_with('/'));
                    assert!(path.text().ends_with('s'));
                    assert!(path.size().x <= width);
                }
                out.textures_delta.clear();
            }
        }
    }

    #[test]
    fn thumb_selection_follows_the_listing_click_rules() {
        use fm_vfs::{Entry, EntryKind, VfsUri};
        let items: Vec<Entry> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|n| {
                let uri = VfsUri::parse(&format!("/t/{n}")).expect("uri");
                let mut e = Entry::new(uri, EntryKind::File);
                e.name = (*n).to_string();
                e
            })
            .collect();
        let names = |s: &ThumbSel| s.names.iter().cloned().collect::<Vec<_>>().join("");
        let mut s = ThumbSel::default();
        s.click(&items, 1, false, false);
        assert_eq!(names(&s), "b");
        s.click(&items, 3, false, true);
        assert_eq!(names(&s), "bd");
        s.click(&items, 3, false, true);
        assert_eq!(names(&s), "b");
        s.click(&items, 4, true, false);
        assert_eq!(names(&s), "de");
        s.click(&items, 9, false, false);
        assert_eq!(names(&s), "de");
    }
    use fm_vfs::VfsUri;
    use std::time::{Duration, Instant};

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1, 0xe2, 0x21, 0xbc, 0x33, 0, 0, 0, 0,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    const GOOD_PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0xd, 0xa, 0x1a, 0xa, 0x0, 0x0, 0x0, 0xd, 0x49, 0x48, 0x44, 0x52,
        0x0, 0x0, 0x0, 0x1, 0x0, 0x0, 0x0, 0x1, 0x8, 0x6, 0x0, 0x0, 0x0, 0x1f, 0x15, 0xc4, 0x89,
        0x0, 0x0, 0x0, 0xd, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x64, 0x62, 0xfe, 0xf,
        0x0, 0x1, 0x14, 0x1, 0x6, 0xd6, 0xb9, 0xa6, 0x45, 0x0, 0x0, 0x0, 0x0, 0x49, 0x45, 0x4e,
        0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn frame(s: &mut Session, pane: PaneId, tokens: &Tokens) {
        let ctx = egui::Context::default();
        crate::theme::apply_to(&ctx, tokens);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| show(ui, s, pane, tokens));
        });
        out.textures_delta.clear();
    }

    #[test]
    fn every_body_renders_in_both_palettes() {
        let dir = tempfile::tempdir().expect("tmp");
        let files: [(&str, &[u8]); 6] = [
            ("a.rs", b"fn main() {}\n"),
            ("b.json", br#"{"a":[1,2]}"#),
            ("c.md", b"# Title\n\nsome *text* and `code`\n\n- item\n"),
            ("d.png", PNG),
            ("e.bin", &[0u8, 1, 2, 3, 255]),
            (
                "f.log",
                b"[2024-05-01 10:00:00] main.ERROR: boom\n#0 stack\n[2024-05-01 10:00:01] main.INFO: ok\n",
            ),
        ];
        for (n, d) in files {
            std::fs::write(dir.path().join(n), d).expect("write");
        }
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
        let files_pane = s.layout.focused();
        s.layout
            .state_mut(files_pane)
            .expect("pane")
            .navigate(uri.clone());
        s.ensure_read(uri).expect("read");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.pane_view_ready(files_pane) {
            assert!(Instant::now() < deadline);
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        s.dispatch("view.preview.toggle", &json!({})).expect("open");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| {
                s.layout
                    .state(*p)
                    .is_some_and(|st| st.kind == PaneKind::Preview)
            })
            .expect("preview pane");
        for name in ["a.rs", "b.json", "c.md", "d.png", "e.bin", "f.log"] {
            let (entries, ix) = s.pane_view(files_pane).expect("view");
            let row = ix
                .order
                .iter()
                .position(|i| entries[*i as usize].name == name)
                .expect("row");
            let st = s.layout.state_mut(files_pane).expect("pane");
            st.listing.on_current = false;
            st.listing.on_parent = false;
            st.listing.focused_row = row;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                s.poll();
                let slot = s.preview_slot(pane).expect("slot");
                let done = slot.subject.as_ref().is_some_and(|e| e.name == name)
                    && matches!(slot.body, Body::Static(_) | Body::Log(_));
                if done {
                    break;
                }
                assert!(Instant::now() < deadline, "{name} never rendered");
                std::thread::sleep(Duration::from_millis(5));
            }
            for tokens in [Tokens::dark(), Tokens::light()] {
                frame(&mut s, pane, &tokens);
                if name == "f.log" {
                    use egui_kittest::{Harness, kittest::Queryable};

                    let Body::Log(view) = &s.preview_slot(pane).expect("slot").body else {
                        panic!("expected log preview");
                    };
                    let mut harness =
                        Harness::builder()
                            .with_size(vec2(1000.0, 400.0))
                            .build_ui(|ui| {
                                crate::theme::apply_to(ui.ctx(), &tokens);
                                ui.style_mut().interaction.tooltip_delay = 0.0;
                                let mut actions = Vec::new();
                                ui.horizontal(|ui| log_toolbar(ui, pane, view, &mut actions));
                                assert!(actions.is_empty(), "hover must not change filters");
                            });
                    harness.get_by_role(egui::accesskit::Role::ComboBox).hover();
                    harness.run();
                    assert_eq!(harness.get_all_by_label("Minimum severity").count(), 2);
                    harness.get_by_label("Warning shows Warning, Error and Critical.");
                    harness.get_by_label("Only the preview changes, not the file.");
                    harness.get_by_label(".*").hover();
                    harness.run();
                    harness.get_by_label("Regular-expression search");
                    harness.get_by_label("Example: error|warning");
                    harness.get_by_label("Expand").hover();
                    harness.run();
                    harness.get_by_label("Expand log records");
                    harness.get_by_label("Turn off to collapse records to their first line.");
                    harness.get_by_label("Live tail").hover();
                    harness.run();
                    harness.get_by_label("Unchanged: no file content downloaded. Growth: reads only appended bytes from the last offset.");
                    harness.get_by_label(
                        "Set the polling interval and initial tail size in Preferences > Preview.",
                    );
                    for (button, help) in [
                        (
                            "Time",
                            "Choose a quick range or custom From and To dates and times (UTC).",
                        ),
                        ("Mark", "Set a marker below the last record (Ctrl+Shift+M)"),
                        ("Clear", "No active filters to clear."),
                    ] {
                        harness.get_by_label(button).hover();
                        harness.run();
                        harness.get_by_label(help);
                    }
                }
            }
        }
        s.dispatch("preview.toggle_pretty", &json!({})).ok();
        s.dispatch("preview.log.fold", &json!({"mode": "expand"}))
            .ok();
        frame(&mut s, pane, &Tokens::dark());
        if s.dispatch("preview.log.mark", &json!({})).is_ok() {
            for tokens in [Tokens::dark(), Tokens::light()] {
                frame(&mut s, pane, &tokens);
            }
        }
    }

    #[test]
    fn thumbnail_grid_asks_for_its_window_and_renders_in_both_palettes() {
        use fm_engine::thumbs::ThumbState;
        let dir = tempfile::tempdir().expect("tmp");
        let photos = dir.path().join("photos");
        std::fs::create_dir_all(photos.join("sub")).expect("dirs");
        std::fs::write(photos.join("a.png"), GOOD_PNG).expect("a");
        std::fs::write(photos.join("b.png"), GOOD_PNG).expect("b");
        std::fs::write(photos.join("c.txt"), b"hi").expect("c");
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
        let files_pane = s.layout.focused();
        s.layout
            .state_mut(files_pane)
            .expect("pane")
            .navigate(uri.clone());
        s.ensure_read(uri).expect("read");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.pane_view_ready(files_pane) {
            assert!(Instant::now() < deadline);
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        s.dispatch("view.preview.toggle", &json!({})).expect("open");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| {
                s.layout
                    .state(*p)
                    .is_some_and(|st| st.kind == PaneKind::Preview)
            })
            .expect("preview pane");
        let st = s.layout.state_mut(files_pane).expect("pane");
        st.listing.on_current = false;
        st.listing.on_parent = false;
        st.listing.focused_row = 0;
        let items = loop {
            s.poll();
            let slot = s.preview_slot(pane).expect("slot");
            if let Body::Static(p) = &slot.body
                && let Preview::Dir(d) = &**p
            {
                break d.items.clone();
            }
            assert!(Instant::now() < deadline, "dir never rendered");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(items.len(), 4);
        s.dispatch("preview.thumbs.toggle", &json!({})).expect("on");
        for tokens in [Tokens::dark(), Tokens::light()] {
            let a = items.iter().find(|e| e.name == "a.png").expect("a.png");
            while !matches!(s.thumbs.get(a, 160), ThumbState::Ready(_)) {
                frame(&mut s, pane, &tokens);
                s.poll();
                assert!(Instant::now() < deadline, "thumbnail never arrived");
                std::thread::sleep(Duration::from_millis(5));
            }
            frame(&mut s, pane, &tokens);
        }
        s.dispatch("preview.thumbs.size", &json!({"size": "large"}))
            .expect("size");
        frame(&mut s, pane, &Tokens::dark());
        assert!(
            s.preview_slot(pane)
                .is_some_and(|sl| sl.thumb_window.is_some())
        );

        // A focused grid moves its cursor with the keys and tells the listing to stand down.
        let tokens = Tokens::dark();
        let ctx = egui::Context::default();
        crate::theme::apply_to(&ctx, &tokens);
        let state_id = Id::NULL.with(("pv-thumb-state", pane));
        let run = |s: &mut Session, ctx: &egui::Context, key: Option<egui::Key>| {
            let mut input = egui::RawInput::default();
            if let Some(key) = key {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut out = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| show(ui, s, pane, &tokens));
            });
            out.textures_delta.clear();
        };
        run(&mut s, &ctx, None);
        run(&mut s, &ctx, None);
        run(&mut s, &ctx, None);
        // Following the files pane's cursor onto a folder must not steal its keyboard.
        assert_ne!(s.layout.focused(), pane);
        assert!(
            !grid_has_keyboard(&ctx),
            "grid stole focus from the files pane"
        );
        run(&mut s, &ctx, Some(egui::Key::End));
        assert!(!grid_has_keyboard(&ctx));
        let _ = s.layout.focus_pane(pane);
        run(&mut s, &ctx, None);
        run(&mut s, &ctx, None);
        assert!(grid_has_keyboard(&ctx), "grid never got focus");
        run(&mut s, &ctx, Some(egui::Key::ArrowRight));
        let cursor = |ctx: &egui::Context| {
            ctx.data(|d| d.get_temp::<(usize, f32, f32)>(state_id))
                .map(|t| t.0)
        };
        assert_eq!(cursor(&ctx), Some(1));
        assert!(grid_has_keyboard(&ctx));
        run(&mut s, &ctx, Some(egui::Key::PageDown));
        assert_eq!(cursor(&ctx), Some(3));
        run(&mut s, &ctx, Some(egui::Key::Home));
        assert_eq!(cursor(&ctx), Some(0));
        // Focusing another pane hands the keys back.
        let _ = s.layout.focus_pane(files_pane);
        run(&mut s, &ctx, None);
        assert!(!grid_has_keyboard(&ctx));
    }

    #[test]
    fn a_video_renders_with_controls_in_both_palettes() {
        let Some(ffmpeg) = fm_preview::video::find_tool("ffmpeg") else {
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let clip = dir.path().join("clip.mp4");
        let made = std::process::Command::new(ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=320x240:rate=25:duration=3")
            .args(["-pix_fmt", "yuv420p", "-y"])
            .arg(&clip)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return;
        }
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
        let files_pane = s.layout.focused();
        s.layout
            .state_mut(files_pane)
            .expect("pane")
            .navigate(uri.clone());
        s.ensure_read(uri).expect("read");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.pane_view_ready(files_pane) {
            assert!(Instant::now() < deadline);
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        s.dispatch("view.preview.toggle", &json!({})).expect("open");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| {
                s.layout
                    .state(*p)
                    .is_some_and(|st| st.kind == PaneKind::Preview)
            })
            .expect("preview pane");
        let st = s.layout.state_mut(files_pane).expect("pane");
        st.listing.on_current = false;
        st.listing.on_parent = false;
        st.listing.focused_row = 0;
        for tokens in [Tokens::dark(), Tokens::light()] {
            // The first frames report the pane width; the decoder then delivers a frame.
            let until = Instant::now() + Duration::from_secs(10);
            loop {
                frame(&mut s, pane, &tokens);
                s.poll();
                if s.preview_slot(pane)
                    .and_then(|sl| sl.video.as_ref())
                    .is_some_and(|v| v.frame.is_some())
                {
                    break;
                }
                assert!(Instant::now() < until, "no video frame");
                std::thread::sleep(Duration::from_millis(10));
            }
            frame(&mut s, pane, &tokens);
        }
        s.dispatch("preview.video.play", &json!({"on": false, "pane": pane}))
            .expect("pause");
        frame(&mut s, pane, &Tokens::dark());
    }
}
