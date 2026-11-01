//! Diff pane body: aligned side-by-side columns (or a unified list) over the engine's model.
//! The UI only draws; actions dispatch `diff.*` commands.

use egui::text::LayoutJob;
use egui::{Color32, FontId, ScrollArea, TextFormat, Ui, vec2};
use fm_engine::Session;
use fm_engine::diff_cmds::{DiffStatus, Outcome, Row, Tag};
use fm_layout::{DiffMode, PaneId};
use serde_json::json;

use crate::theme::Tokens;

/// Below this width the split view falls back to the unified list.
const MIN_SPLIT: f32 = 480.0;
const NUM_W: f32 = 44.0;

pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let Some(state) = s.layout.state(pane).and_then(|st| st.diff.clone()) else {
        ui.label("No comparison. Use Compare on a file.");
        return;
    };
    let status = s.diff_view(pane).map(|v| v.status.clone());

    ui.horizontal(|ui| {
        for (side, color) in [
            (&state.left, tokens.git_removed),
            (&state.right, tokens.git_added),
        ] {
            let full = side.label();
            let short = match side {
                fm_layout::DiffSide::File(p) => std::path::Path::new(p)
                    .file_name()
                    .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned()),
                _ => full.clone(),
            };
            ui.label(egui::RichText::new(short).color(color))
                .on_hover_text(full);
            if std::ptr::eq(side, &state.left) {
                ui.label("vs");
            }
        }
    });
    ui.horizontal(|ui| {
        for (id, label) in [
            ("diff.prev", "Prev"),
            ("diff.next", "Next"),
            ("diff.swap", "Swap"),
            ("diff.mode", "Split/Unified"),
            ("diff.eol.toggle", "Ignore EOL"),
            ("diff.reload", "Reload"),
        ] {
            if ui
                .add_enabled(s.command_enabled(id), egui::Button::new(label))
                .clicked()
            {
                s.layout.focus_pane(pane).ok();
                s.dispatch_or_notice(id, &json!({}));
            }
        }
    });
    ui.separator();

    match status {
        None | Some(DiffStatus::Loading) => {
            ui.label("Comparing…");
        }
        Some(DiffStatus::Failed(e)) => {
            ui.colored_label(tokens.error, e);
        }
        Some(DiffStatus::Ready(outcome)) => match outcome.as_ref() {
            Outcome::Binary { same } => {
                ui.label(if *same {
                    "Binary files are identical"
                } else {
                    "Binary files differ"
                });
            }
            Outcome::Text(model) => {
                if model.identical() {
                    ui.label("Files are identical");
                }
                ui.label(
                    egui::RichText::new(format!(
                        "{} changes · +{} -{}",
                        model.changes.len(),
                        model.added,
                        model.removed
                    ))
                    .color(tokens.text_muted),
                );
                let unified = state.mode == DiffMode::Unified || ui.available_width() < MIN_SPLIT;
                let target = model
                    .changes
                    .get(state.current)
                    .copied()
                    .map(|row| (state.current, row));
                rows(ui, pane, &model.rows, unified, target, tokens);
            }
        },
    }
}

fn rows(
    ui: &mut Ui,
    pane: PaneId,
    rows: &[Row],
    unified: bool,
    target: Option<(usize, usize)>,
    tokens: &Tokens,
) {
    let font = FontId::monospace(tokens.font_size);
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 2.0;
    let seen = ui.make_persistent_id(egui::IdSalt::new(("diff-current", pane)));
    let last: Option<usize> = ui.ctx().data(|d| d.get_temp(seen));
    let mut area = ScrollArea::both()
        .id_salt(("diff-scroll", pane))
        .auto_shrink(false);
    if let Some((index, row)) = target
        && last != Some(index)
    {
        let visual = if unified {
            unified_index(rows, row)
        } else {
            row
        };
        area = area.vertical_scroll_offset((visual as f32 * row_h - row_h * 3.0).max(0.0));
    }
    ui.ctx().data_mut(|d| match target {
        Some((i, _)) => {
            d.insert_temp(seen, i);
        }
        None => d.remove::<usize>(seen),
    });

    let total = if unified {
        unified_index(rows, rows.len())
    } else {
        rows.len()
    };
    area.show_rows(ui, row_h, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if unified {
            let mut at = 0;
            for row in rows {
                let n = unified_len(row);
                if at + n > range.start && at < range.end {
                    unified_row(ui, row, row_h, &font, tokens);
                }
                at += n;
                if at >= range.end {
                    break;
                }
            }
        } else {
            let half = ((ui.available_width() - 4.0) / 2.0).max(100.0);
            for row in rows.get(range).unwrap_or_default() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    side(ui, row, false, half, row_h, &font, tokens);
                    side(ui, row, true, half, row_h, &font, tokens);
                });
            }
        }
    });
}

fn unified_len(row: &Row) -> usize {
    match row.tag {
        Tag::Equal | Tag::Delete | Tag::Insert => 1,
        Tag::Replace => 2,
    }
}

/// Visual line index of row `row` in the unified list.
fn unified_index(rows: &[Row], row: usize) -> usize {
    rows.iter().take(row).map(unified_len).sum()
}

fn backgrounds(tag: Tag, right: bool, tokens: &Tokens) -> (Color32, Color32) {
    match (tag, right) {
        (Tag::Delete | Tag::Replace, false) => (tokens.diff_del_bg, tokens.diff_del_word),
        (Tag::Insert | Tag::Replace, true) => (tokens.diff_add_bg, tokens.diff_add_word),
        _ => (tokens.bg.gamma_multiply(0.0), tokens.bg.gamma_multiply(0.0)),
    }
}

fn line_job(
    text: &str,
    marks: &[(u32, u32)],
    word_bg: Color32,
    font: &FontId,
    tokens: &Tokens,
) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |bg: Option<Color32>| TextFormat {
        font_id: font.clone(),
        color: tokens.text,
        background: bg.unwrap_or_default(),
        ..Default::default()
    };
    let mut at = 0usize;
    for &(a, b) in marks {
        let (a, b) = (a as usize, (b as usize).min(text.len()));
        let (Some(plain), Some(hit)) = (text.get(at..a.max(at)), text.get(a.max(at)..b.max(a)))
        else {
            continue;
        };
        job.append(plain, 0.0, fmt(None));
        job.append(hit, 0.0, fmt(Some(word_bg)));
        at = b.max(a).max(at);
    }
    job.append(text.get(at..).unwrap_or_default(), 0.0, fmt(None));
    job
}

fn side(
    ui: &mut Ui,
    row: &Row,
    right: bool,
    width: f32,
    row_h: f32,
    font: &FontId,
    tokens: &Tokens,
) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), egui::Sense::hover());
    let line = if right { &row.right } else { &row.left };
    let (bg, word) = backgrounds(row.tag, right, tokens);
    let painter = ui.painter_at(rect);
    match line {
        Some(l) => {
            painter.rect_filled(rect, 0.0, bg);
            painter.text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                l.no.to_string(),
                FontId::monospace(tokens.font_size * 0.85),
                tokens.text_muted,
            );
            let galley =
                ui.fonts_mut(|f| f.layout_job(line_job(&l.text, &l.marks, word, font, tokens)));
            painter.galley(rect.left_top() + vec2(NUM_W, 1.0), galley, tokens.text);
        }
        None => {
            painter.rect_filled(rect, 0.0, tokens.diff_del_bg.gamma_multiply(0.3));
        }
    }
}

fn unified_row(ui: &mut Ui, row: &Row, row_h: f32, font: &FontId, tokens: &Tokens) {
    let one = |ui: &mut Ui, right: bool| {
        let line = if right { &row.right } else { &row.left };
        let Some(l) = line else { return };
        let (rect, _) =
            ui.allocate_exact_size(vec2(ui.available_width(), row_h), egui::Sense::hover());
        let (bg, word) = backgrounds(row.tag, right, tokens);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, bg);
        let sign = match (row.tag, right) {
            (Tag::Equal, _) => ' ',
            (_, false) => '-',
            (_, true) => '+',
        };
        painter.text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            format!("{:>5} {sign}", l.no),
            FontId::monospace(tokens.font_size * 0.85),
            tokens.text_muted,
        );
        let galley =
            ui.fonts_mut(|f| f.layout_job(line_job(&l.text, &l.marks, word, font, tokens)));
        painter.galley(
            rect.left_top() + vec2(NUM_W + 16.0, 1.0),
            galley,
            tokens.text,
        );
    };
    match row.tag {
        Tag::Equal | Tag::Insert => one(ui, row.tag == Tag::Insert),
        Tag::Delete => one(ui, false),
        Tag::Replace => {
            one(ui, false);
            one(ui, true);
        }
    }
}
