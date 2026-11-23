//! Table view of delimited text (change `add-table-preview`): the toolbar choices, the
//! filter bar, a virtualized grid with its own vertical scrolling (design D9, so any row
//! count stays exact) and the Record panel. The UI only draws and dispatches
//! `preview.table.*` commands; rows come from the engine's [`TableView`].

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Align2, CursorIcon, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Ui, vec2};
use fm_engine::preview_table::{TableView, ViewStatus};
use fm_layout::PaneId;
use fm_preview::table::delimiter_label;
use fm_preview::table::query::Op;
use serde_json::json;

use super::Action;
use crate::theme::Tokens;

/// Narrowest and widest automatic column width, in characters.
const MIN_CHARS: f32 = 4.0;
const MAX_CHARS: f32 = 40.0;
/// Narrowest width a column can be dragged to, in points.
const MIN_WIDTH: f32 = 28.0;
/// Scrollbar thickness.
const BAR: f32 = 10.0;

/// Identity of the shown document, so per-file UI state resets with the file.
fn doc_id(v: &TableView) -> usize {
    Arc::as_ptr(&v.doc) as usize
}

/// Grid presentation state (scroll position, dragged widths), per pane.
#[derive(Clone, Default)]
struct GridUi {
    doc: usize,
    /// First visible view row.
    top: u64,
    /// Horizontal scroll in points over the data columns.
    x: f32,
    /// Wheel movement not yet turned into whole rows.
    wheel: f32,
    /// Widths the user dragged.
    widths: HashMap<usize, f32>,
    /// Column being resized and its width when the drag started.
    drag: Option<(usize, f32)>,
    /// Last `scroll_to` sequence handled.
    seen_scroll: u64,
    /// Rows the grid showed in the last frame (for Page Up/Down).
    page: u64,
}

/// Filter bar text fields, per pane.
#[derive(Clone, Default)]
struct BarUi {
    doc: usize,
    query: String,
    regex: bool,
    column: Option<usize>,
    pattern_error: Option<String>,
    goto: String,
    /// Condition being added: column, operator, value.
    cond: Option<(usize, Op, String)>,
    focus_filter: bool,
}

fn grid_id(pane: PaneId) -> Id {
    Id::NULL.with(("pv-table-grid", pane))
}

fn bar_id(pane: PaneId) -> Id {
    Id::NULL.with(("pv-table-bar", pane))
}

fn bar_state(ui: &Ui, pane: PaneId, v: &TableView) -> BarUi {
    let st: BarUi = ui
        .data_mut(|d| d.get_temp(bar_id(pane)))
        .unwrap_or_default();
    if st.doc == doc_id(v) {
        return st;
    }
    BarUi {
        doc: doc_id(v),
        query: v.filter.query.clone(),
        regex: v.filter.regex,
        column: v.filter.column,
        ..Default::default()
    }
}

/// Ctrl+F in a table focuses its quick filter.
pub(super) fn focus_filter(ui: &Ui, pane: PaneId) {
    ui.data_mut(|d| {
        let mut st: BarUi = d.get_temp(bar_id(pane)).unwrap_or_default();
        st.focus_filter = true;
        d.insert_temp(bar_id(pane), st);
    });
}

/// `12345678` → `12,345,678`.
fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// View options in the preview toolbar: Preview/Source, delimiter, header, Record.
pub(super) fn toolbar_options(ui: &mut Ui, v: &TableView, source: bool, actions: &mut Vec<Action>) {
    if ui
        .add(egui::Button::selectable(!source, "Preview"))
        .on_hover_text(if source {
            "Show the table"
        } else {
            "Show the file's text"
        })
        .clicked()
    {
        actions.push(("preview.toggle_source", json!({})));
    }
    if source {
        return;
    }
    let d = &v.doc;
    let delimiter = match d.options.delimiter {
        None => format!("Auto ({})", delimiter_label(d.detected.delimiter)),
        Some(c) => delimiter_label(c).to_string(),
    };
    let combo = egui::ComboBox::from_id_salt("pv-table-delimiter")
        .selected_text(format!("Delimiter: {delimiter}"))
        .show_ui(ui, |ui| {
            for (label, id, value) in [
                ("Auto", "auto", None),
                ("Comma", "comma", Some(b',')),
                ("Semicolon", "semicolon", Some(b';')),
                ("Tab", "tab", Some(b'\t')),
                ("Pipe", "pipe", Some(b'|')),
            ] {
                if ui
                    .selectable_label(d.options.delimiter == value, label)
                    .clicked()
                {
                    actions.push(("preview.table.dialect", json!({ "delimiter": id })));
                }
            }
        })
        .response;
    combo.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Delimiter"));
    combo.on_hover_text("How fields are separated. Auto detects it from the file.");
    let header = match d.options.header {
        None => format!("Auto ({})", if d.detected_header { "yes" } else { "no" }),
        Some(true) => "Yes".into(),
        Some(false) => "No".into(),
    };
    let combo = egui::ComboBox::from_id_salt("pv-table-header")
        .selected_text(format!("Header: {header}"))
        .show_ui(ui, |ui| {
            for (label, id, value) in [
                ("Auto", "auto", None),
                ("First row is the header", "yes", Some(true)),
                ("No header", "no", Some(false)),
            ] {
                if ui
                    .selectable_label(d.options.header == value, label)
                    .clicked()
                {
                    actions.push(("preview.table.dialect", json!({ "header": id })));
                }
            }
        })
        .response;
    combo.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Header row"));
    combo.on_hover_text("Whether the first row holds the column names.");
    if ui
        .add(egui::Button::selectable(v.record, "Record"))
        .on_hover_text("Show the selected row as name and value pairs (Enter)")
        .clicked()
    {
        actions.push(("preview.table.record", json!({})));
    }
}

/// The filter bar under the toolbar: quick filter, scope, Regex, condition chips, the
/// condition editor, Go to row and the row counts.
pub(super) fn filter_bar(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    v: &TableView,
    actions: &mut Vec<Action>,
) {
    let mut st = bar_state(ui, pane, v);
    let columns = v.columns();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let edit = ui.add(
            egui::TextEdit::singleline(&mut st.query)
                .hint_text("Filter rows")
                .desired_width(180.0),
        );
        if std::mem::take(&mut st.focus_filter) {
            edit.request_focus();
        }
        let mut changed = edit.changed();
        let scope = st
            .column
            .map_or_else(|| "All columns".to_string(), |c| v.doc.column_name(c));
        let combo = egui::ComboBox::from_id_salt(("pv-table-scope", pane))
            .selected_text(format!("in {scope}"))
            .height(320.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(st.column.is_none(), "All columns")
                    .clicked()
                {
                    st.column = None;
                    changed = true;
                }
                for c in (0..columns).filter(|c| !v.hidden.contains(c)) {
                    if ui
                        .selectable_label(st.column == Some(c), v.doc.column_name(c))
                        .clicked()
                    {
                        st.column = Some(c);
                        changed = true;
                    }
                }
            })
            .response;
        combo
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, "Filter column"));
        if ui
            .add(egui::Button::selectable(st.regex, "Regex"))
            .on_hover_text("Match the text as a regular expression")
            .clicked()
        {
            st.regex = !st.regex;
            changed = true;
        }
        if changed {
            st.pattern_error = if st.regex {
                fm_engine::preview_table_cmds::pattern_error(&st.query)
            } else {
                None
            };
            // An invalid pattern is reported here and leaves the rows as they are.
            if st.pattern_error.is_none() {
                actions.push((
                    "preview.table.filter",
                    json!({ "query": st.query, "regex": st.regex, "column": st.column }),
                ));
            }
        }
        if let Some(e) = &st.pattern_error {
            ui.label(RichText::new(e).color(tokens.error));
        }
        for (i, c) in v.filter.conditions.iter().enumerate() {
            let text = if c.op.takes_value() {
                format!(
                    "{} {} {}  ×",
                    v.doc.column_name(c.column),
                    c.op.label(),
                    c.value
                )
            } else {
                format!("{} {}  ×", v.doc.column_name(c.column), c.op.label())
            };
            if ui
                .add(egui::Button::new(text).small())
                .on_hover_text("Remove this condition")
                .clicked()
            {
                actions.push(("preview.table.condition.remove", json!({ "index": i })));
            }
        }
        if !v.filter.is_empty()
            && ui
                .button("Clear")
                .on_hover_text("Remove every filter")
                .clicked()
        {
            st.query.clear();
            st.pattern_error = None;
            actions.push(("preview.table.filter.clear", json!({})));
        }
        let goto = ui.add(
            egui::TextEdit::singleline(&mut st.goto)
                .hint_text("Go to row")
                .desired_width(110.0),
        );
        if goto.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            if let Ok(row) = st.goto.trim().replace(',', "").parse::<u64>() {
                actions.push(("preview.table.goto", json!({ "row": row })));
            }
            st.goto.clear();
        }
        status(ui, tokens, v);
    });
    if let Some((column, op, value)) = &mut st.cond {
        let mut close = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(RichText::new(v.doc.column_name(*column)).strong());
            egui::ComboBox::from_id_salt(("pv-table-op", pane))
                .selected_text(op.label())
                .show_ui(ui, |ui| {
                    for o in Op::ALL {
                        ui.selectable_value(op, o, o.label());
                    }
                });
            let mut submit = false;
            if op.takes_value() {
                let field = ui.add(
                    egui::TextEdit::singleline(value)
                        .hint_text("Value")
                        .desired_width(140.0),
                );
                submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            }
            let ready = !op.takes_value() || !value.is_empty();
            if ui.add_enabled(ready, egui::Button::new("Add")).clicked() || (submit && ready) {
                actions.push((
                    "preview.table.condition.add",
                    json!({ "column": column, "op": op.id(), "value": value }),
                ));
                close = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
        if close {
            st.cond = None;
        }
    }
    ui.data_mut(|d| d.insert_temp(bar_id(pane), st));
}

fn status(ui: &mut Ui, tokens: &Tokens, v: &TableView) {
    let total = v.total_rows();
    let mut parts = Vec::new();
    match v.view_status() {
        ViewStatus::Natural => parts.push(format!("{} rows", count(total))),
        ViewStatus::Ready => {
            parts.push(format!("{} of {} rows", count(v.row_count()), count(total)))
        }
        ViewStatus::Running { progress } => {
            parts.push(format!("filtering {:.0}%", progress * 100.0));
        }
        ViewStatus::Failed(e) => {
            ui.label(RichText::new(e).color(tokens.error));
        }
    }
    parts.push(format!("{} columns", count(v.columns() as u64)));
    if let Some(p) = v.indexing() {
        parts.push(format!("indexing {:.0}%", p * 100.0));
    }
    let malformed = v.malformed();
    if malformed > 0 {
        parts.push(format!("{} malformed records", count(malformed)));
    }
    if let Some(n) = v.note() {
        parts.push(n);
    }
    ui.label(tokens.muted(parts.join(" · ")));
    if let Some(f) = v.failure() {
        ui.label(RichText::new(f).color(tokens.error));
    }
}

/// The table body: the grid and, when shown, the Record panel.
pub(super) fn body(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    v: &TableView,
    actions: &mut Vec<Action>,
) {
    let avail = ui.available_rect_before_wrap();
    ui.allocate_rect(avail, Sense::hover());
    let (grid_rect, record_rect) = if v.record {
        if avail.width() >= 560.0 {
            let w = (avail.width() * 0.38).clamp(240.0, 520.0);
            let split = avail.max.x - w;
            (
                Rect::from_min_max(avail.min, Pos2::new(split - 4.0, avail.max.y)),
                Some(Rect::from_min_max(
                    Pos2::new(split + 4.0, avail.min.y),
                    avail.max,
                )),
            )
        } else {
            let h = (avail.height() * 0.4).max(120.0);
            let split = avail.max.y - h;
            (
                Rect::from_min_max(avail.min, Pos2::new(avail.max.x, split - 4.0)),
                Some(Rect::from_min_max(
                    Pos2::new(avail.min.x, split + 4.0),
                    avail.max,
                )),
            )
        }
    } else {
        (avail, None)
    };
    grid(ui, pane, tokens, v, grid_rect, actions);
    if let Some(rect) = record_rect {
        ui.painter().rect_filled(rect, 0.0, tokens.panel);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
        record_panel(&mut child, tokens, v);
    }
}

fn record_panel(ui: &mut Ui, tokens: &Tokens, v: &TableView) {
    let Some((row, _)) = v.selected else {
        ui.label(tokens.muted("Select a cell to see its whole row here."));
        return;
    };
    let Some((number, data)) = v.row(row) else {
        ui.label(tokens.muted("Reading…"));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
        return;
    };
    ui.label(RichText::new(format!("Row {}", count(number + 1))).strong());
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("pv-table-record")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for c in (0..v.columns()).filter(|c| !v.hidden.contains(c)) {
                ui.label(tokens.muted(v.doc.column_name(c)));
                let value = data.cell(c);
                if value.is_empty() {
                    ui.label(tokens.muted("(empty)"));
                } else {
                    ui.add(
                        egui::Label::new(RichText::new(value).color(tokens.text))
                            .wrap()
                            .selectable(true),
                    );
                }
                if data.cut.contains(&c) {
                    ui.label(tokens.muted(format!(
                        "Cut at {} KiB; open the file in an editor for the rest.",
                        fm_preview::table::scan::MAX_CELL >> 10
                    )));
                }
                ui.add_space(6.0);
            }
        });
}

/// Width of column `c` in points.
fn width_of(v: &TableView, st: &GridUi, c: usize, char_w: f32, pad: f32) -> f32 {
    if let Some(w) = st.widths.get(&c) {
        return *w;
    }
    let chars = v
        .doc
        .columns
        .get(c)
        .map_or(10.0, |col| f32::from(col.width))
        .clamp(MIN_CHARS, MAX_CHARS);
    chars * char_w + 2.0 * pad + 12.0
}

/// First line of a cell, and whether it had more lines.
fn first_line(s: &str) -> (&str, bool) {
    match s.split_once(['\n', '\r']) {
        Some((first, _)) => (first, true),
        None => (s, false),
    }
}

#[allow(clippy::too_many_lines)]
fn grid(
    ui: &mut Ui,
    pane: PaneId,
    tokens: &Tokens,
    v: &TableView,
    rect: Rect,
    actions: &mut Vec<Action>,
) {
    let id = grid_id(pane);
    let mut st: GridUi = ui.data_mut(|d| d.get_temp(id)).unwrap_or_default();
    if st.doc != doc_id(v) {
        st = GridUi {
            doc: doc_id(v),
            ..Default::default()
        };
    }
    let font: FontId = egui::TextStyle::Body.resolve(ui.style());
    let painter = ui.painter_at(rect);
    let char_w = painter
        .layout_no_wrap("0".into(), font.clone(), tokens.text)
        .size()
        .x
        .max(4.0);
    let pad = (char_w * 0.6).round();
    let row_h = (font.size * 1.55)
        .round()
        .max(tokens.row_height.min(font.size * 2.2));
    let header_h = row_h + 4.0;
    let rows_total = v.row_count();
    let visible_cols: Vec<usize> = (0..v.columns()).filter(|c| !v.hidden.contains(c)).collect();
    let widths: Vec<f32> = visible_cols
        .iter()
        .map(|c| width_of(v, &st, *c, char_w, pad))
        .collect();
    let content_w: f32 = widths.iter().sum();
    // Row numbers are file row numbers, so a filter showing rows from far down the file
    // needs the width of the file's count, not of how many rows match.
    let digits = count(v.total_rows().max(rows_total).max(1)).len() as f32;
    let num_w = (digits * char_w + 2.0 * pad).max(3.0 * char_w);
    let data_left = rect.min.x + num_w;
    let data_w = (rect.max.x - BAR - data_left).max(1.0);
    let need_hbar = content_w > data_w;
    let body_top = rect.min.y + header_h;
    let body_bottom = rect.max.y - if need_hbar { BAR } else { 0.0 };
    let page = (((body_bottom - body_top) / row_h).floor().max(1.0)) as u64;
    st.page = page;
    let max_top = rows_total.saturating_sub(page);

    // Scroll requests from commands (select, Go to row): row and column into view.
    if let Some((row, seq)) = v.scroll_to
        && seq != st.seen_scroll
    {
        st.seen_scroll = seq;
        if row < st.top {
            st.top = row;
        } else if row >= st.top + page {
            st.top = row + 1 - page;
        }
        if let Some((_, col)) = v.selected
            && let Some(pos) = visible_cols.iter().position(|c| *c == col)
        {
            let left: f32 = widths.iter().take(pos).sum();
            let right = left + widths.get(pos).copied().unwrap_or(0.0);
            if left < st.x {
                st.x = left;
            } else if right > st.x + data_w {
                st.x = right - data_w;
            }
        }
    }

    // Wheel: vertical in whole rows, horizontal (or Shift+wheel) in points.
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        let (delta, shift) = ui.input(|i| (i.smooth_scroll_delta, i.modifiers.shift));
        let (dx, dy) = if shift && delta.x == 0.0 {
            (delta.y, 0.0)
        } else {
            (delta.x, delta.y)
        };
        if dy != 0.0 {
            st.wheel -= dy / row_h;
            let whole = st.wheel.trunc();
            st.wheel -= whole;
            if whole > 0.0 {
                st.top = st.top.saturating_add(whole as u64);
            } else if whole < 0.0 {
                st.top = st.top.saturating_sub((-whole) as u64);
            }
        }
        if dx != 0.0 {
            st.x -= dx;
        }
        if dx != 0.0 || dy != 0.0 {
            ui.input_mut(|i| i.smooth_scroll_delta = egui::Vec2::ZERO);
        }
    }
    st.top = st.top.min(max_top);
    st.x = st.x.clamp(0.0, (content_w - data_w).max(0.0));

    // Keyboard: the preview pane has focus and no text field does.
    let can_key = ui
        .data(|d| d.get_temp::<bool>(Id::NULL.with("pv-can-page")))
        .unwrap_or(false);
    if can_key && rows_total > 0 && !visible_cols.is_empty() {
        keyboard(ui, v, &visible_cols, page, actions);
    }

    let rows = v.rows(st.top, (page + 1) as usize);
    let waiting = rows.iter().any(Option::is_none);

    // Background, stripes and selection.
    painter.rect_filled(rect, 0.0, tokens.bg);
    let stripe = tokens.hover_bg.linear_multiply(0.45);
    let sel_row_bg = tokens.selection_bg.linear_multiply(0.35);
    for (i, _) in rows.iter().enumerate() {
        let view_row = st.top + i as u64;
        let y = body_top + i as f32 * row_h;
        if y >= body_bottom {
            break;
        }
        let r = Rect::from_min_max(
            Pos2::new(rect.min.x, y),
            Pos2::new(rect.max.x - BAR, (y + row_h).min(body_bottom)),
        );
        if v.selected.is_some_and(|(sr, _)| sr == view_row) {
            painter.rect_filled(r, 0.0, sel_row_bg);
        } else if !view_row.is_multiple_of(2) {
            painter.rect_filled(r, 0.0, stripe);
        }
    }

    // Cells.
    let data_clip = Rect::from_min_max(
        Pos2::new(data_left, body_top),
        Pos2::new(rect.max.x - BAR, body_bottom),
    );
    let cells = painter.with_clip_rect(data_clip);
    let grid_line = tokens.border.linear_multiply(0.5);
    let mut x = data_left - st.x;
    let mut col_x: Vec<(usize, f32, f32)> = Vec::new();
    for (k, c) in visible_cols.iter().enumerate() {
        let w = widths.get(k).copied().unwrap_or(60.0);
        if x + w >= data_left && x <= rect.max.x - BAR {
            col_x.push((*c, x, w));
        }
        x += w;
    }
    for (c, cx, w) in &col_x {
        let numeric = v.doc.column_kind(*c).numeric();
        cells.vline(cx + w, body_top..=body_bottom, Stroke::new(1.0, grid_line));
        for (i, row) in rows.iter().enumerate() {
            let y = body_top + i as f32 * row_h;
            if y >= body_bottom {
                break;
            }
            let Some((_, data)) = row else { continue };
            let view_row = st.top + i as u64;
            let cell_rect = Rect::from_min_size(Pos2::new(*cx, y), vec2(*w, row_h));
            let selected = v.selected == Some((view_row, *c));
            if selected {
                cells.rect_filled(cell_rect.shrink(1.0), 2.0, tokens.selection_bg);
            }
            let (line, more) = first_line(data.cell(*c));
            if line.is_empty() && !more {
                continue;
            }
            let color = if selected {
                tokens.selection_text
            } else {
                tokens.text
            };
            let inner = cell_rect.shrink2(vec2(pad, 0.0));
            let clip = cells.with_clip_rect(inner.intersect(data_clip));
            let galley = clip.layout_no_wrap(line.to_string(), font.clone(), color);
            let marker_w = if more { char_w * 1.6 } else { 0.0 };
            let tx = if numeric && !more {
                (inner.max.x - galley.size().x).max(inner.min.x)
            } else {
                inner.min.x
            };
            let ty = inner.center().y - galley.size().y / 2.0;
            clip.galley(Pos2::new(tx, ty), galley, color);
            if more {
                let mx = (tx
                    + clip
                        .layout_no_wrap(line.to_string(), font.clone(), color)
                        .size()
                        .x
                    + char_w * 0.4)
                    .min(inner.max.x - marker_w);
                clip.text(
                    Pos2::new(mx, inner.center().y),
                    Align2::LEFT_CENTER,
                    "¶",
                    font.clone(),
                    tokens.text_muted,
                );
            }
        }
    }

    // Row numbers (sticky on the left).
    let num_rect = Rect::from_min_max(
        Pos2::new(rect.min.x, body_top),
        Pos2::new(data_left, body_bottom),
    );
    painter.rect_filled(num_rect, 0.0, tokens.panel);
    let nums = painter.with_clip_rect(num_rect);
    for (i, row) in rows.iter().enumerate() {
        let y = body_top + i as f32 * row_h;
        if y >= body_bottom {
            break;
        }
        let text = match row {
            Some((n, _)) => count(n + 1),
            None => "…".into(),
        };
        nums.text(
            Pos2::new(data_left - pad, y + row_h / 2.0),
            Align2::RIGHT_CENTER,
            text,
            font.clone(),
            tokens.text_muted,
        );
    }
    painter.vline(
        data_left,
        rect.min.y..=body_bottom,
        Stroke::new(1.0, tokens.border),
    );

    // Header (sticky on top).
    let header_rect = Rect::from_min_max(rect.min, Pos2::new(rect.max.x - BAR, body_top));
    painter.rect_filled(header_rect, 0.0, tokens.panel);
    painter.hline(
        rect.min.x..=rect.max.x,
        body_top,
        Stroke::new(1.0, tokens.border),
    );
    let header_clip = Rect::from_min_max(
        Pos2::new(data_left, rect.min.y),
        Pos2::new(rect.max.x - BAR, body_top),
    );
    let heads = painter.with_clip_rect(header_clip);
    let shift = ui.input(|i| i.modifiers.shift);
    for (c, cx, w) in &col_x {
        let cell = Rect::from_min_size(Pos2::new(*cx, rect.min.y), vec2(*w, header_h));
        let name = v.doc.column_name(*c);
        let sort_pos = v.sort.iter().position(|k| k.column == *c);
        let arrow_w = if sort_pos.is_some() {
            char_w * 2.2
        } else {
            0.0
        };
        let inner = cell.shrink2(vec2(pad, 0.0));
        let clip = heads.with_clip_rect(
            Rect::from_min_max(inner.min, Pos2::new(inner.max.x - arrow_w, inner.max.y))
                .intersect(header_clip),
        );
        clip.text(
            Pos2::new(inner.min.x, inner.center().y),
            Align2::LEFT_CENTER,
            &name,
            FontId::new(font.size, font.family.clone()),
            tokens.text,
        );
        if let Some(pos) = sort_pos {
            let descending = v.sort.get(pos).is_some_and(|k| k.descending);
            let center = Pos2::new(inner.max.x - arrow_w * 0.55, inner.center().y);
            let s = char_w * 0.45;
            let points = if descending {
                vec![
                    center + vec2(-s, -s * 0.6),
                    center + vec2(s, -s * 0.6),
                    center + vec2(0.0, s * 0.7),
                ]
            } else {
                vec![
                    center + vec2(-s, s * 0.6),
                    center + vec2(s, s * 0.6),
                    center + vec2(0.0, -s * 0.7),
                ]
            };
            heads.add(egui::Shape::convex_polygon(
                points,
                tokens.accent,
                Stroke::NONE,
            ));
            if v.sort.len() > 1 {
                heads.text(
                    Pos2::new(inner.max.x, inner.center().y),
                    Align2::RIGHT_CENTER,
                    (pos + 1).to_string(),
                    FontId::new(font.size * 0.7, font.family.clone()),
                    tokens.text_muted,
                );
            }
        }
        heads.vline(
            cx + w,
            rect.min.y..=body_top,
            Stroke::new(1.0, tokens.border),
        );
        // Click to sort, right-click for the column menu.
        let click_rect = Rect::from_min_max(cell.min, Pos2::new(cell.max.x - 4.0, cell.max.y))
            .intersect(header_clip);
        if click_rect.width() > 0.0 {
            let resp = ui.interact(click_rect, id.with(("head", *c)), Sense::click());
            resp.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Sort by {name}"))
            });
            if resp.clicked() {
                let current = sort_pos.and_then(|p| v.sort.get(p));
                let next = match current {
                    None => "asc",
                    Some(k) if !k.descending => "desc",
                    Some(_) => "none",
                };
                let (order, add) = if shift {
                    (next, true)
                } else if v.sort.len() > 1 && current.is_some() {
                    ("asc", false)
                } else {
                    (next, false)
                };
                actions.push((
                    "preview.table.sort",
                    json!({ "column": c, "order": order, "add": add }),
                ));
            }
            let kind = v.doc.column_kind(*c);
            let resp = resp.on_hover_text(format!(
                "{name}\n{}. Click to sort, Shift+click to add a sort column, right-click for more.",
                match kind.label() {
                    "text" => "Text",
                    "numbers" => "Numbers",
                    _ => "Whole numbers",
                }
            ));
            resp.context_menu(|ui| {
                if ui.button("Sort ascending").clicked() {
                    actions.push(("preview.table.sort", json!({ "column": c, "order": "asc" })));
                    ui.close();
                }
                if ui.button("Sort descending").clicked() {
                    actions.push((
                        "preview.table.sort",
                        json!({ "column": c, "order": "desc" }),
                    ));
                    ui.close();
                }
                if sort_pos.is_some() && ui.button("Remove from sort").clicked() {
                    actions.push((
                        "preview.table.sort",
                        json!({ "column": c, "order": "none", "add": true }),
                    ));
                    ui.close();
                }
                ui.separator();
                if ui.button("Filter this column…").clicked() {
                    ui.data_mut(|d| {
                        let mut bar: BarUi = d.get_temp(bar_id(pane)).unwrap_or_default();
                        bar.cond = Some((
                            *c,
                            if kind.numeric() {
                                Op::Greater
                            } else {
                                Op::Contains
                            },
                            String::new(),
                        ));
                        d.insert_temp(bar_id(pane), bar);
                    });
                    ui.close();
                }
                if ui.button("Quick filter in this column").clicked() {
                    ui.data_mut(|d| {
                        let mut bar: BarUi = d.get_temp(bar_id(pane)).unwrap_or_default();
                        bar.column = Some(*c);
                        bar.focus_filter = true;
                        d.insert_temp(bar_id(pane), bar);
                    });
                    ui.close();
                }
                ui.separator();
                if ui.button("Hide column").clicked() {
                    actions.push(("preview.table.column.hide", json!({ "column": c })));
                    ui.close();
                }
                if !v.hidden.is_empty() && ui.button("Show all columns").clicked() {
                    actions.push(("preview.table.columns.show_all", json!({})));
                    ui.close();
                }
                if ui.button("Copy column names").clicked() {
                    actions.push(("preview.table.copy", json!({ "what": "header" })));
                    ui.close();
                }
            });
        }
        // Drag the right edge to resize.
        let edge = Rect::from_center_size(
            Pos2::new(cx + w, rect.min.y + header_h / 2.0),
            vec2(8.0, header_h),
        )
        .intersect(header_clip);
        if edge.width() > 0.0 {
            let resp = ui.interact(edge, id.with(("edge", *c)), Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            if resp.drag_started() {
                st.drag = Some((*c, *w));
            }
            if resp.dragged()
                && let Some((dc, start)) = st.drag
                && dc == *c
            {
                let total = resp.total_drag_delta().map_or(0.0, |d| d.x);
                st.widths.insert(*c, (start + total).max(MIN_WIDTH));
            }
            if resp.drag_stopped() {
                st.drag = None;
            }
            if resp.double_clicked() {
                st.widths.remove(c);
            }
        }
    }
    painter.rect_filled(
        Rect::from_min_max(rect.min, Pos2::new(data_left, body_top)),
        0.0,
        tokens.panel,
    );

    // Clicks on cells: select; double-click opens the Record panel; right-click menu.
    let cells_resp = ui.interact(data_clip, id.with("cells"), Sense::click());
    let hit = |pos: Pos2| -> Option<(u64, usize)> {
        let i = ((pos.y - body_top) / row_h).floor();
        if i < 0.0 {
            return None;
        }
        let row = st.top + i as u64;
        if row >= rows_total {
            return None;
        }
        let col = col_x
            .iter()
            .find(|(_, cx, w)| pos.x >= *cx && pos.x < cx + w)
            .map(|(c, _, _)| *c)?;
        Some((row, col))
    };
    if (cells_resp.clicked() || cells_resp.secondary_clicked() || cells_resp.double_clicked())
        && let Some(pos) = cells_resp.interact_pointer_pos()
        && let Some((row, col)) = hit(pos)
    {
        if v.selected != Some((row, col)) {
            actions.push(("preview.table.select", json!({ "row": row, "column": col })));
        }
        if cells_resp.double_clicked() && !v.record {
            actions.push(("preview.table.record", json!({ "show": true })));
        }
    }
    cells_resp.context_menu(|ui| {
        let selected = v.selected.is_some();
        if ui
            .add_enabled(selected, egui::Button::new("Copy cell"))
            .clicked()
        {
            actions.push(("preview.table.copy", json!({ "what": "cell" })));
            ui.close();
        }
        if ui
            .add_enabled(selected, egui::Button::new("Copy row"))
            .clicked()
        {
            actions.push(("preview.table.copy", json!({ "what": "row" })));
            ui.close();
        }
        if ui
            .button(if v.record {
                "Hide record"
            } else {
                "Show record"
            })
            .clicked()
        {
            actions.push(("preview.table.record", json!({})));
            ui.close();
        }
    });

    // Scrollbars.
    let vtrack = Rect::from_min_max(
        Pos2::new(rect.max.x - BAR, body_top),
        Pos2::new(rect.max.x, body_bottom),
    );
    painter.rect_filled(
        Rect::from_min_max(Pos2::new(rect.max.x - BAR, rect.min.y), rect.max),
        0.0,
        tokens.panel,
    );
    if rows_total > page {
        let frac = page as f32 / rows_total as f32;
        let thumb_h = (vtrack.height() * frac).max(24.0).min(vtrack.height());
        let travel = (vtrack.height() - thumb_h).max(1.0);
        let pos = st.top as f64 / max_top.max(1) as f64;
        let thumb = Rect::from_min_size(
            Pos2::new(vtrack.min.x + 2.0, vtrack.min.y + (pos as f32) * travel),
            vec2(BAR - 4.0, thumb_h),
        );
        let resp = ui.interact(vtrack, id.with("vbar"), Sense::click_and_drag());
        if (resp.dragged() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let t = ((p.y - vtrack.min.y - thumb_h / 2.0) / travel).clamp(0.0, 1.0);
            st.top = ((t as f64) * max_top as f64).round() as u64;
        }
        let color = if resp.hovered() || resp.dragged() {
            tokens.text_muted
        } else {
            tokens.border
        };
        painter.rect_filled(thumb, 3.0, color);
    }
    if need_hbar {
        let htrack = Rect::from_min_max(
            Pos2::new(data_left, body_bottom),
            Pos2::new(rect.max.x - BAR, rect.max.y),
        );
        painter.rect_filled(htrack, 0.0, tokens.panel);
        let frac = data_w / content_w;
        let thumb_w = (htrack.width() * frac).max(24.0).min(htrack.width());
        let travel = (htrack.width() - thumb_w).max(1.0);
        let max_x = (content_w - data_w).max(1.0);
        let thumb = Rect::from_min_size(
            Pos2::new(htrack.min.x + st.x / max_x * travel, htrack.min.y + 2.0),
            vec2(thumb_w, BAR - 4.0),
        );
        let resp = ui.interact(htrack, id.with("hbar"), Sense::click_and_drag());
        if (resp.dragged() || resp.clicked())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let t = ((p.x - htrack.min.x - thumb_w / 2.0) / travel).clamp(0.0, 1.0);
            st.x = t * max_x;
        }
        let color = if resp.hovered() || resp.dragged() {
            tokens.text_muted
        } else {
            tokens.border
        };
        painter.rect_filled(thumb, 3.0, color);
    }
    if rows_total == 0 {
        painter.text(
            data_clip.center(),
            Align2::CENTER_CENTER,
            if v.filter.is_empty() {
                "No rows"
            } else {
                "No rows match the filter"
            },
            font,
            tokens.text_muted,
        );
    }
    if waiting || v.is_busy() {
        // The engine wakes the UI when rows arrive; this keeps progress moving without it.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    ui.data_mut(|d| d.insert_temp(id, st));
}

/// Arrow keys, Page Up/Down, Home/End, Ctrl+C and Enter in the grid.
fn keyboard(ui: &mut Ui, v: &TableView, cols: &[usize], page: u64, actions: &mut Vec<Action>) {
    use egui::{Key, Modifiers};
    let last_row = v.row_count().saturating_sub(1);
    let (row, col) = v
        .selected
        .unwrap_or((0, cols.first().copied().unwrap_or(0)));
    let ci = cols.iter().position(|c| *c == col).unwrap_or(0);
    let mut target = None;
    let mut used = false;
    ui.input_mut(|i| {
        let mut key = |m: Modifiers, k: Key| i.consume_key(m, k);
        if key(Modifiers::NONE, Key::ArrowDown) {
            target = Some((row.saturating_add(1).min(last_row), ci));
        } else if key(Modifiers::NONE, Key::ArrowUp) {
            target = Some((row.saturating_sub(1), ci));
        } else if key(Modifiers::NONE, Key::ArrowRight) {
            target = Some((row, (ci + 1).min(cols.len().saturating_sub(1))));
        } else if key(Modifiers::NONE, Key::ArrowLeft) {
            target = Some((row, ci.saturating_sub(1)));
        } else if key(Modifiers::NONE, Key::PageDown) {
            target = Some((row.saturating_add(page).min(last_row), ci));
        } else if key(Modifiers::NONE, Key::PageUp) {
            target = Some((row.saturating_sub(page), ci));
        } else if key(Modifiers::COMMAND, Key::Home) {
            target = Some((0, ci));
        } else if key(Modifiers::COMMAND, Key::End) {
            target = Some((last_row, ci));
        } else if key(Modifiers::NONE, Key::Home) {
            target = Some((row, 0));
        } else if key(Modifiers::NONE, Key::End) {
            target = Some((row, cols.len().saturating_sub(1)));
        } else if key(Modifiers::NONE, Key::Enter) {
            actions.push(("preview.table.record", json!({})));
            used = true;
        } else if v.selected.is_some() {
            let mut copy = key(Modifiers::COMMAND, Key::C);
            i.events.retain(|e| {
                if matches!(e, egui::Event::Copy) {
                    copy = true;
                    false
                } else {
                    true
                }
            });
            if copy {
                actions.push(("preview.table.copy", json!({ "what": "cell" })));
                used = true;
            }
        }
    });
    if let Some((r, c)) = target
        && let Some(col) = cols.get(c)
    {
        actions.push(("preview.table.select", json!({ "row": r, "column": col })));
        used = true;
    }
    if used {
        ui.data_mut(|d| d.insert_temp(Id::NULL.with("pv-page-used"), true));
    }
}
