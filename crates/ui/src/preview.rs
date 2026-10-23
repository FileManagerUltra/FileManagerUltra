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
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let mut actions: Vec<Action> = Vec::new();
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
        let wrap_pretty = |ui: &mut Ui, actions: &mut Vec<Action>| {
            if ui.selectable_label(slot.wrap, "Wrap").clicked() {
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
        Preview::Dir(d) => dir_card(ui, tokens, d),
        Preview::Meta(m) => {
            meta_card(ui, tokens, m);
            if let Some(poster) = &m.poster {
                image(ui, pane, tokens, p, poster, Zoom::Fit);
            }
        }
        Preview::Error(e) => {
            ui.add_space(12.0);
            ui.label(RichText::new("Cannot preview").color(tokens.error).strong());
            ui.label(e);
        }
        Preview::Log(_) => centered(ui, tokens, "Log"),
    }
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
    if wrap {
        job.wrap.max_width = width;
    } else {
        job.wrap.max_width = f32::INFINITY;
    }
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
    let area = ScrollArea::both()
        .id_salt(("pv-text", pane))
        .auto_shrink(false);
    if wrap {
        area.show(ui, |ui| {
            let width = ui.available_width();
            for (i, line) in d.lines.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.label(
                        RichText::new(format!("{:>gutter$}", i + 1))
                            .font(mono(tokens))
                            .color(tokens.text_muted),
                    );
                    ui.label(spans_job(
                        tokens,
                        line,
                        true,
                        width - 12.0 * gutter as f32 - 20.0,
                    ));
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

fn dir_card(ui: &mut Ui, tokens: &Tokens, d: &DirDoc) {
    ui.add_space(8.0);
    let mut rows = vec![
        ("Files".to_string(), d.files.to_string()),
        ("Folders".to_string(), d.dirs.to_string()),
        ("Other".to_string(), d.other.to_string()),
        ("Total size".to_string(), format!("{} bytes", d.bytes)),
    ];
    if let Some((name, _)) = &d.newest {
        rows.push(("Newest".to_string(), name.clone()));
    }
    card_rows(ui, tokens, "pv-dir", &rows);
    if d.truncated {
        ui.label(tokens.muted("Only the first entries were counted"));
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use fm_vfs::VfsUri;
    use std::time::{Duration, Instant};

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0,
        0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0, 0, 0, 2, 0, 1, 0xe2, 0x21, 0xbc, 0x33, 0, 0, 0, 0,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
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
}
