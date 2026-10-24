//! Preview pane body: renders the engine's `Body` for the file under the cursor.
//! The UI only draws; every action is dispatched as a command (`preview.*`).

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{FontId, Id, RichText, ScrollArea, Sense, TextFormat, Ui, vec2};
use fm_engine::Session;
use fm_engine::preview_cmds::{Body, LogView, Zoom};
use fm_layout::{PaneId, PaneKind, PreviewSource};
use fm_preview::log::{Level, LogDoc, LogRecord};
use fm_preview::markdown::{Inline, MdBlock};
use fm_preview::{DirDoc, HexDoc, ImageDoc, MetaDoc, Preview, Span, StructDoc, TextDoc};
use serde_json::{Value, json};

use crate::theme::Tokens;

type Action = (&'static str, Value);

/// Per-pane text fields of the log filter bar.
#[derive(Clone, Default)]
struct LogUi {
    query: String,
    regex: bool,
    from: String,
    to: String,
}

fn mono(tokens: &Tokens) -> FontId {
    FontId::monospace(tokens.font_size)
}

/// Render the preview pane body.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<fm_vfs::VfsUri> {
    let mut actions: Vec<Action> = Vec::new();
    let mut open: Option<fm_vfs::VfsUri> = None;
    let mut published: Option<fm_engine::file_cmds::GridSelection> = None;
    toolbar(ui, s, pane, &mut actions);
    ui.separator();
    if let Some(slot) = s.preview_slot(pane) {
        match &slot.body {
            Body::Empty => centered(ui, tokens, "Select a file to preview it"),
            Body::Loading => {
                ui.centered_and_justified(|ui| ui.spinner());
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(100));
            }
            Body::Static(p) if slot.thumbs && matches!(&**p, Preview::Dir(_)) => {
                if let Preview::Dir(d) = &**p {
                    thumb_grid(
                        ui,
                        s,
                        pane,
                        tokens,
                        d,
                        slot.thumb_size,
                        slot.thumb_window,
                        &mut actions,
                        &mut open,
                        slot.subject.as_ref().map(|e| e.uri.clone()),
                        &mut published,
                    );
                }
            }
            Body::Static(p) if slot.video.is_some() => {
                if let (Preview::Meta(m), Some(v)) = (&**p, &slot.video) {
                    video_body(ui, pane, tokens, p, m, v, &mut actions);
                }
            }
            Body::Static(p) => {
                let p = p.clone();
                static_body(ui, pane, tokens, &p, slot.pretty, slot.wrap, slot.zoom);
            }
            Body::Log(v) => log_body(ui, pane, tokens, v, &mut actions),
        }
    }
    for (id, params) in actions {
        let mut params = params;
        if let Some(o) = params.as_object_mut() {
            o.insert("pane".into(), json!(pane));
        }
        s.dispatch_or_notice(id, &params);
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

fn toolbar(ui: &mut Ui, s: &Session, pane: PaneId, actions: &mut Vec<Action>) {
    let Some(slot) = s.preview_slot(pane) else {
        return;
    };
    ui.horizontal(|ui| {
        source_picker(ui, s, pane, actions);
        if let Some(e) = &slot.subject {
            ui.label(RichText::new(&e.name).strong());
        }
        // Right-aligned, framed buttons so view options stand apart from the title.
        ui.add_space(ui.available_width().min(12.0));
        ui.separator();
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.spacing_mut().button_padding = vec2(10.0, 3.0);
        let wrap_pretty = |ui: &mut Ui, actions: &mut Vec<Action>| {
            if ui
                .add(egui::Button::selectable(slot.wrap, "Wrap"))
                .clicked()
            {
                actions.push(("preview.toggle_wrap", json!({})));
            }
        };
        match &slot.body {
            Body::Static(p) => match &**p {
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
                Preview::Text(_) | Preview::Markdown(_) => wrap_pretty(ui, actions),
                Preview::Image(_) => {
                    for (label, mode) in [
                        ("Fit", "fit"),
                        ("100%", "actual"),
                        ("+", "in"),
                        ("−", "out"),
                    ] {
                        if ui.button(label).clicked() {
                            actions.push(("preview.zoom", json!({ "mode": mode })));
                        }
                    }
                }
                Preview::Dir(_) => thumb_toolbar(
                    ui,
                    slot.thumbs,
                    slot.thumb_size,
                    s.prefs.recursive_preview,
                    actions,
                ),
                _ => {}
            },
            Body::Log(v) => log_toolbar(ui, pane, v, actions),
            _ => {}
        }
    });
}

fn source_picker(ui: &mut Ui, s: &Session, pane: PaneId, actions: &mut Vec<Action>) {
    let Some(state) = s.layout.state(pane) else {
        return;
    };
    let files: Vec<PaneId> = s
        .layout
        .panes()
        .into_iter()
        .filter(|p| {
            s.layout
                .state(*p)
                .is_some_and(|st| st.kind == PaneKind::Files)
        })
        .collect();
    let current = match state.preview_source {
        PreviewSource::Follow => "Active pane".to_string(),
        PreviewSource::Pane(id) => format!("Pane {id}"),
    };
    egui::ComboBox::from_id_salt(("pv-source", pane))
        .selected_text(current)
        .width(100.0)
        .show_ui(ui, |ui| {
            if ui
                .selectable_label(
                    state.preview_source == PreviewSource::Follow,
                    "Active pane (follows focus)",
                )
                .clicked()
            {
                actions.push(("pane.preview.source", json!({ "source": "focus" })));
            }
            for id in files {
                let on = state.preview_source == PreviewSource::Pane(id);
                if ui.selectable_label(on, format!("Pane {id}")).clicked() {
                    actions.push(("pane.preview.source", json!({ "source": id })));
                }
            }
        });
}

fn static_body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    p: &Arc<Preview>,
    pretty: bool,
    wrap: bool,
    zoom: Zoom,
) {
    match &**p {
        Preview::Text(d) => text_view(ui, pane, tokens, d, wrap),
        Preview::Structured(d) => structured(ui, pane, tokens, d, pretty, wrap),
        Preview::Markdown(m) => markdown(ui, tokens, &m.blocks, m.truncated),
        Preview::Image(i) => image(ui, pane, tokens, p, i, zoom),
        Preview::Hex(h) => hex(ui, tokens, h),
        Preview::Dir(d) => scrolled(ui, pane, "dir", |ui| dir_card(ui, tokens, d)),
        Preview::Meta(m) => scrolled(ui, pane, "meta", |ui| {
            meta_card(ui, tokens, m);
            if let Some(poster) = &m.poster {
                image(ui, pane, tokens, p, poster, Zoom::Fit);
            }
        }),
        Preview::Error(e) => scrolled(ui, pane, "err", |ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("Cannot preview").color(tokens.error).strong());
            ui.label(e);
        }),
        Preview::Log(_) => centered(ui, tokens, "Log"),
    }
}

/// Cards are taller than a small pane; scroll them instead of overflowing the slot.
fn scrolled(ui: &mut Ui, pane: PaneId, what: &str, add: impl FnOnce(&mut Ui)) {
    ScrollArea::vertical()
        .id_salt(("pv-scroll", what, pane))
        .auto_shrink(false)
        .show(ui, add);
}

fn spans_job(tokens: &Tokens, spans: &[Span], wrap: bool, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for sp in spans {
        job.append(
            &sp.text,
            0.0,
            TextFormat::simple(mono(tokens), tokens.syntax(sp.class)),
        );
    }
    job.wrap.max_width = if wrap { width } else { f32::INFINITY };
    job
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
    let row_h = ui.fonts_mut(|f| f.row_height(&mono(tokens))) + 2.0;
    let area = if wrap {
        ScrollArea::vertical()
    } else {
        ScrollArea::both()
    }
    .id_salt(("pv-text", pane))
    .auto_shrink(false);
    if wrap {
        area.show(ui, |ui| {
            for (i, line) in d.lines.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new(format!("{:>gutter$}", i + 1))
                            .font(mono(tokens))
                            .color(tokens.text_muted),
                    );
                    ui.add(egui::Label::new(spans_job(tokens, line, true, 0.0)).wrap());
                });
            }
        });
    } else {
        area.show_rows(ui, row_h, d.lines.len(), |ui, range| {
            for i in range {
                let Some(line) = d.lines.get(i) else { continue };
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("{:>gutter$}", i + 1))
                            .font(mono(tokens))
                            .color(tokens.text_muted),
                    );
                    ui.add(egui::Label::new(spans_job(tokens, line, false, 0.0)).extend());
                });
            }
        });
    }
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

fn inline_job(tokens: &Tokens, parts: &[Inline], base: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for p in parts {
        let font = if p.code {
            FontId::monospace(base)
        } else {
            FontId::proportional(base)
        };
        let color = if p.link.is_some() {
            tokens.accent
        } else {
            tokens.text
        };
        let mut f = TextFormat::simple(font, color);
        f.italics = p.italic;
        if p.bold {
            f.color = tokens.text;
            f.underline = egui::Stroke::NONE;
        }
        if p.link.is_some() {
            f.underline = egui::Stroke::new(1.0, tokens.accent);
        }
        if p.code {
            f.background = tokens.panel;
        }
        job.append(&p.text, 0.0, f);
    }
    job
}

fn markdown(ui: &mut Ui, tokens: &Tokens, blocks: &[MdBlock], truncated: bool) {
    ScrollArea::vertical()
        .id_salt("pv-md")
        .auto_shrink(false)
        .show(ui, |ui| {
            for b in blocks {
                match b {
                    MdBlock::Heading(l, parts) => {
                        ui.add_space(6.0);
                        let size = tokens.font_size * (2.0 - (*l as f32 - 1.0) * 0.18).max(1.0);
                        ui.label(inline_job(tokens, parts, size));
                    }
                    MdBlock::Para(parts) => {
                        ui.label(inline_job(tokens, parts, tokens.font_size));
                        ui.add_space(4.0);
                    }
                    MdBlock::Code(c) => {
                        egui::Frame::NONE
                            .fill(tokens.panel)
                            .inner_margin(6.0)
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(c.trim_end()).font(mono(tokens)));
                            });
                    }
                    MdBlock::Item(depth, marker, parts) => {
                        ui.horizontal_top(|ui| {
                            ui.add_space(16.0 * *depth as f32);
                            ui.label(marker);
                            ui.label(inline_job(tokens, parts, tokens.font_size));
                        });
                    }
                    MdBlock::Quote(parts) => {
                        ui.horizontal_top(|ui| {
                            ui.label(tokens.muted("▎"));
                            ui.label(inline_job(tokens, parts, tokens.font_size));
                        });
                    }
                    MdBlock::Rule => {
                        ui.separator();
                    }
                }
            }
            if truncated {
                ui.label(tokens.muted("… truncated"));
            }
        });
}

fn image(ui: &mut Ui, pane: PaneId, tokens: &Tokens, p: &Arc<Preview>, img: &ImageDoc, zoom: Zoom) {
    if img.frames.is_empty() {
        centered(ui, tokens, "No pixels to show");
        return;
    }
    let ctx = ui.ctx().clone();
    let id = Id::NULL.with(("pv-tex", pane));
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
    let scale = match zoom {
        Zoom::Fit => (avail.x / natural.x)
            .min(avail.y / natural.y)
            .clamp(0.01, 1.0),
        Zoom::Actual => 1.0,
        Zoom::Scale(v) => v,
    };
    ScrollArea::both()
        .id_salt(("pv-img", pane))
        .auto_shrink(false)
        .max_height(avail.y)
        .show(ui, |ui| {
            ui.add(egui::Image::new((tex.id(), natural * scale)).sense(Sense::hover()));
        });
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

fn hex(ui: &mut Ui, tokens: &Tokens, h: &HexDoc) {
    let row_h = ui.fonts_mut(|f| f.row_height(&mono(tokens))) + 2.0;
    let rows = h.bytes.len().div_ceil(16);
    ScrollArea::both()
        .id_salt("pv-hex")
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
    if (h.bytes.len() as u64) < h.total {
        ui.label(tokens.muted(format!("First {} of {} bytes", h.bytes.len(), h.total)));
    }
}

fn card_rows(ui: &mut Ui, tokens: &Tokens, id: &str, rows: &[(String, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(tokens.muted(k));
                ui.label(v);
                ui.end_row();
            }
        });
}

fn meta_card(ui: &mut Ui, tokens: &Tokens, m: &MetaDoc) {
    ui.add_space(8.0);
    card_rows(ui, tokens, "pv-meta", &m.rows);
    if let Some(h) = &m.hint {
        ui.add_space(8.0);
        ui.label(tokens.muted(h));
    }
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
    ScrollArea::vertical()
        .id_salt(("pv-video", pane))
        .auto_shrink(false)
        .show(ui, |ui| {
            let controls_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y * 2.0;
            let aspect = v.info.height as f32 / v.info.width.max(1) as f32;
            let w = avail
                .x
                .min(((avail.y - controls_h) / aspect.max(0.01)).max(64.0));
            let size = vec2(w, w * aspect);
            if let Some(f) = &v.frame {
                let id = Id::NULL.with(("pv-video-tex", pane));
                let ctx = ui.ctx().clone();
                let cached: Option<(usize, egui::TextureHandle)> = ctx.data_mut(|d| d.get_temp(id));
                let key = Arc::as_ptr(f) as usize;
                let image = || {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [f.width as usize, f.height as usize],
                        &f.rgba,
                    )
                };
                let tex = match cached {
                    Some((k, t)) if k == key => t,
                    Some((_, mut t)) => {
                        t.set(image(), egui::TextureOptions::LINEAR);
                        t
                    }
                    None => ctx.load_texture(
                        format!("pv-video-{pane}"),
                        image(),
                        egui::TextureOptions::LINEAR,
                    ),
                };
                ctx.data_mut(|d| d.insert_temp(id, (key, tex.clone())));
                let (rect, resp) = ui.allocate_exact_size(vec2(avail.x, size.y), Sense::click());
                let at = egui::Rect::from_center_size(rect.center(), size);
                egui::Image::new((tex.id(), size)).paint_at(ui, at);
                if resp.clicked() {
                    actions.push(("preview.video.play", json!({})));
                }
            } else if let Some(poster) = &m.poster {
                image(ui, pane, tokens, p, poster, Zoom::Fit);
            } else {
                ui.allocate_space(vec2(avail.x, size.y.min(120.0)));
            }
            video_controls(ui, pane, v, actions);
            if let Some(e) = &v.error {
                ui.colored_label(tokens.error, e);
            }
            meta_card(ui, tokens, m);
        });
    if v.playing {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn clock(secs: f64) -> String {
    let t = secs.max(0.0) as u64;
    if t >= 3600 {
        format!("{}:{:02}:{:02}", t / 3600, t / 60 % 60, t % 60)
    } else {
        format!("{}:{:02}", t / 60, t % 60)
    }
}

fn video_controls(
    ui: &mut Ui,
    pane: PaneId,
    v: &fm_engine::video_cmds::VideoState,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        let label = if v.playing { "Pause" } else { "Play" };
        if ui.button(label).clicked() {
            actions.push(("preview.video.play", json!({})));
        }
        let dur = v.info.duration.max(0.0);
        let id = Id::NULL.with(("pv-video-seek", pane));
        let dragging: Option<f64> = ui.data(|d| d.get_temp(id));
        let mut value = dragging.unwrap_or(v.pos).clamp(0.0, dur);
        ui.label(format!("{} / {}", clock(value), clock(dur)));
        ui.spacing_mut().slider_width = (ui.available_width() - 16.0).max(60.0);
        let resp = ui.add_enabled(
            dur > 0.0,
            egui::Slider::new(&mut value, 0.0..=dur.max(0.1)).show_value(false),
        );
        if resp.dragged() {
            ui.data_mut(|d| d.insert_temp(id, value));
        } else {
            ui.data_mut(|d| d.remove::<f64>(id));
            if resp.changed() || resp.drag_stopped() {
                actions.push(("preview.video.seek", json!({ "to": value })));
            }
        }
    });
}

fn thumb_toolbar(ui: &mut Ui, on: bool, size: usize, recursive: bool, actions: &mut Vec<Action>) {
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

#[allow(clippy::too_many_arguments)]
fn thumb_grid(
    ui: &mut Ui,
    s: &Session,
    pane: PaneId,
    tokens: &Tokens,
    d: &DirDoc,
    size: usize,
    asked: Option<(usize, usize)>,
    actions: &mut Vec<Action>,
    open: &mut Option<fm_vfs::VfsUri>,
    dir: Option<fm_vfs::VfsUri>,
    published: &mut Option<fm_engine::file_cmds::GridSelection>,
) {
    use fm_engine::thumbs::ThumbState;
    use fm_preview::thumbs::EDGES;

    if d.items.is_empty() {
        centered(ui, tokens, "(empty folder)");
        return;
    }
    let edge = EDGES.get(size).copied().unwrap_or(EDGES[0]);
    let gap = tokens.padding;
    let label_h = ui.fonts_mut(|f| f.row_height(&tokens.font())) + 2.0;
    let width = ui.available_width();
    let cell = (edge as f32).min((width - gap).max(32.0));
    let cols = (((width + gap) / (cell + gap)).floor() as usize).max(1);
    let rows = d.items.len().div_ceil(cols);
    let row_h = cell + label_h + gap;

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
    let count = d.items.len();
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
    if ctx.data(|dt| dt.get_temp::<bool>(want_id)).unwrap_or(false) {
        if ctx.memory(|m| m.has_focus(grid_id)) {
            ctx.data_mut(|dt| dt.insert_temp(want_id, false));
        } else {
            ctx.memory_mut(|m| m.request_focus(grid_id));
        }
    }
    ctx.data_mut(|dt| dt.insert_temp(shown_id, pass));
    cursor = cursor.min(count - 1);
    let mut scroll_to = None;
    // Registering the widget keeps egui from dropping focus as "disappeared".
    ui.interact(ui.max_rect(), grid_id, Sense::focusable_noninteractive());
    let active = ctx.memory_mut(|m| {
        let on = m.has_focus(grid_id);
        if on {
            m.set_focus_lock_filter(grid_id, ARROW_KEYS);
        }
        on
    });
    ctx.data_mut(|dt| dt.insert_temp(Id::NULL.with("pv-grid-active"), active));
    if active {
        let page_rows = ((view_h / row_h).floor() as usize).saturating_sub(1).max(1);
        let mut target = cursor;
        ui.input_mut(|i| {
            use egui::{Key, Modifiers};
            if i.consume_key(Modifiers::COMMAND, Key::A) {
                sel.names = d.items.iter().map(|e| e.name.clone()).collect();
            }
            if i.consume_key(Modifiers::NONE, Key::Enter)
                && let Some(entry) = d.items.get(cursor)
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
            if key(Key::ArrowUp) && target >= cols {
                target -= cols;
            }
            if key(Key::ArrowDown) && target + cols < count {
                target += cols;
            }
            if key(Key::PageUp) {
                target = target.saturating_sub(page_rows * cols);
            }
            if key(Key::PageDown) {
                target = (target + page_rows * cols).min(count - 1);
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
            sel.click(&d.items, cursor, ui.input(|i| i.modifiers.shift), false);
            // Minimal scroll: only move when the cursor row leaves the viewport.
            let top = (cursor / cols) as f32 * row_h;
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
    let out = area.show_rows(ui, row_h, rows, |ui, range| {
        ui.spacing_mut().item_spacing = vec2(gap, 0.0);
        window = Some((range.start * cols, range.len() * cols));
        for row in range {
            ui.horizontal(|ui| {
                for col in 0..cols {
                    let Some(entry) = d.items.get(row * cols + col) else {
                        break;
                    };
                    let index = row * cols + col;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(cell, row_h - gap), Sense::click_and_drag());
                    if resp.drag_started() {
                        if !sel.names.contains(&entry.name) {
                            cursor = index;
                            sel.click(&d.items, index, false, false);
                        }
                        let sources: Vec<_> = d
                            .items
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
                            sel.click(&d.items, index, m.shift, m.command);
                        } else if !sel.names.contains(&entry.name) {
                            sel.click(&d.items, index, false, false);
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
                    let mut job =
                        LayoutJob::simple(entry.name.clone(), tokens.font(), tokens.text, cell);
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
            });
        }
    });
    offset = out.state.clamped_offset().y;
    ctx.data_mut(|dt| dt.insert_temp(state_id, (cursor, offset, out.inner_rect.height())));
    if let Some(dir) = dir {
        *published = Some(fm_engine::file_cmds::GridSelection {
            pane,
            dir,
            items: d
                .items
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
        ui.label(tokens.muted(format!("Showing the first {} entries", d.items.len())));
    }
}

const SUBDIRS_SHOWN: usize = 12;

fn dir_card(ui: &mut Ui, tokens: &Tokens, d: &DirDoc) {
    ui.add_space(8.0);
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
    let mut subdirs: Vec<(&String, &(usize, u64))> = d.subdirs.iter().collect();
    subdirs.sort_by(|a, b| b.1.1.cmp(&a.1.1).then(a.0.cmp(b.0)));
    let more = subdirs.len().saturating_sub(SUBDIRS_SHOWN);
    for (name, (n, bytes)) in subdirs.into_iter().take(SUBDIRS_SHOWN) {
        rows.push((
            format!("{name}/"),
            format!(
                "{} ({n} {})",
                crate::listing::human_size(*bytes),
                if *n == 1 { "file" } else { "files" }
            ),
        ));
    }
    if more > 0 {
        rows.push(("…".to_string(), format!("and {more} more folders")));
    }
    if d.recursive {
        rows.push(("Scope".to_string(), "all subfolders".to_string()));
    }
    card_rows(ui, tokens, "pv-dir", &rows);
    if d.unreadable > 0 {
        ui.label(tokens.muted(format!("{} folder(s) could not be read", d.unreadable)));
    }
    if d.scanning {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(tokens.muted("Still scanning; totals so far"));
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
        ui.label(tokens.muted(why));
    }
}

// ---- log mode ----

fn log_toolbar(ui: &mut Ui, pane: PaneId, v: &LogView, actions: &mut Vec<Action>) {
    let id = Id::NULL.with(("pv-logui", pane));
    let mut st: LogUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    let before = st.clone();

    let current = v.filter.min_level.map_or("All", Level::label);
    egui::ComboBox::from_id_salt(("pv-level", pane))
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
        });
    ui.add(
        egui::TextEdit::singleline(&mut st.query)
            .hint_text("Search")
            .desired_width(120.0),
    );
    ui.checkbox(&mut st.regex, ".*")
        .on_hover_text("Regular expression");
    if st.query != before.query || st.regex != before.regex {
        actions.push((
            "preview.log.search",
            json!({ "query": st.query, "regex": st.regex }),
        ));
    }
    ui.menu_button("Time", |ui| {
        ui.label("From (YYYY-MM-DD HH:MM)");
        ui.text_edit_singleline(&mut st.from);
        ui.label("To");
        ui.text_edit_singleline(&mut st.to);
        if ui.button("Apply").clicked() {
            actions.push((
                "preview.log.time_range",
                json!({ "from": st.from, "to": st.to }),
            ));
        }
        if ui.button("Clear").clicked() {
            st.from.clear();
            st.to.clear();
            actions.push(("preview.log.time_range", json!({})));
        }
    });
    ui.data_mut(|d| d.insert_temp(id, st));

    let (can_follow, access, more) = match v.doc.try_read() {
        Ok(d) => (d.can_follow(), d.format.is_access(), d.more_before),
        Err(_) => (false, false, false),
    };
    if ui
        .add_enabled(can_follow, egui::Button::selectable(v.follow, "Live tail"))
        .on_disabled_hover_text("This log cannot be followed")
        .clicked()
    {
        actions.push(("preview.log.follow", json!({})));
    }
    if access && ui.selectable_label(v.columns, "Columns").clicked() {
        actions.push(("preview.log.columns", json!({})));
    }
    if ui.selectable_label(v.expand_all, "Expand").clicked() {
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
    if more && ui.button("Load earlier").clicked() {
        actions.push(("preview.log.earlier", json!({})));
    }
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
        (0..shown)
            .filter_map(record_at)
            .flat_map(|r| {
                let lines = doc.records.get(r).map_or(1, |rec| {
                    if v.expand_all || v.expanded.contains(&rec.key()) {
                        rec.text.lines().count().max(1)
                    } else {
                        1
                    }
                });
                (0..lines).map(move |l| (r, l))
            })
            .collect()
    });
    let body_rows = rows.as_ref().map_or(shown, Vec::len);
    let end_mark = doc.marks.contains(&(doc.base + total as i64));
    let count = body_rows + usize::from(end_mark);
    let row_h = ui.fonts_mut(|f| f.row_height(&mono(tokens))) + 2.0;

    let out = ScrollArea::both()
        .id_salt(("pv-log", pane))
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show_rows(ui, row_h, count, |ui, range| {
            for i in range {
                if i >= body_rows {
                    marker_row(ui, tokens, &doc, doc.marks.len(), row_h);
                    continue;
                }
                let (r, line) = match &rows {
                    Some(rows) => match rows.get(i) {
                        Some(x) => *x,
                        None => continue,
                    },
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
                log_row(ui, tokens, &doc, v, rec, line, columns, row_h, actions);
            }
        });
    if v.follow
        && out
            .inner_rect
            .contains(ui.input(|i| i.pointer.hover_pos()).unwrap_or_default())
        && ui.input(|i| i.smooth_scroll_delta.y > 0.0)
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
        tokens.accent,
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
    line: usize,
    columns: bool,
    row_h: f32,
    actions: &mut Vec<Action>,
) {
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width().max(900.0), row_h), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    let painter = ui.painter_at(rect);
    let font = mono(tokens);
    let color = if rec.orphan {
        tokens.text_muted
    } else {
        tokens.level(rec.level)
    };
    let folded = !v.expand_all && !v.expanded.contains(&rec.key());
    let extra = rec.extra_lines();

    if columns && doc.format.is_access() && rec.fields.len() >= 6 {
        let w = rect.width();
        let url_w = (w - 560.0).max(200.0);
        let xs = [
            0.0,
            160.0,
            290.0,
            350.0,
            350.0 + url_w,
            350.0 + url_w + 50.0,
        ];
        for (i, text) in rec.fields.iter().take(6).enumerate() {
            let c = match i {
                4 => rec.status.map_or(tokens.text, |s| tokens.status(s)),
                0 | 5 => tokens.text_muted,
                _ => tokens.text,
            };
            let clip = egui::Rect::from_min_max(
                rect.min + vec2(xs[i], 0.0),
                egui::pos2(
                    rect.min.x + xs.get(i + 1).copied().unwrap_or(w) - 6.0,
                    rect.max.y,
                ),
            );
            painter.with_clip_rect(clip).text(
                clip.left_center(),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                c,
            );
        }
        return;
    }

    let text = rec.text.lines().nth(line).unwrap_or("");
    let mut x = rect.min.x + 4.0;
    if line == 0 && extra > 0 {
        let g = painter.text(
            egui::pos2(x, rect.center().y),
            egui::Align2::LEFT_CENTER,
            if folded { ">" } else { "v" },
            font.clone(),
            tokens.text_muted,
        );
        x = g.right() + 6.0;
    } else if extra > 0 {
        x += 16.0;
    }
    let g = painter.text(
        egui::pos2(x, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        font.clone(),
        if line > 0 { tokens.text_muted } else { color },
    );
    if line == 0 && folded && extra > 0 {
        painter.text(
            egui::pos2(g.right() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("+{extra} lines"),
            font,
            tokens.text_muted,
        );
    }
    if extra > 0 && resp.clicked() {
        actions.push(("preview.log.fold", json!({ "record": rec.key() })));
    }
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
        let mut run = |ctx: &egui::Context, key: Option<egui::Key>| {
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
                egui::CentralPanel::default().show(ctx, |ui| show(ui, &mut s, pane, &tokens));
            });
            out.textures_delta.clear();
        };
        run(&ctx, None);
        run(&ctx, None);
        run(&ctx, None);
        assert!(grid_has_keyboard(&ctx), "grid never got focus");
        run(&ctx, Some(egui::Key::ArrowRight));
        let cursor = |ctx: &egui::Context| {
            ctx.data(|d| d.get_temp::<(usize, f32, f32)>(state_id))
                .map(|t| t.0)
        };
        assert_eq!(cursor(&ctx), Some(1));
        assert!(grid_has_keyboard(&ctx));
        run(&ctx, Some(egui::Key::PageDown));
        assert_eq!(cursor(&ctx), Some(3));
        run(&ctx, Some(egui::Key::Home));
        assert_eq!(cursor(&ctx), Some(0));
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
