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

/// Draw the diff of a Preview pane. The pane toolbar (Preview/Diff toggle) is the
/// preview's; this draws the revision line, the status and the rows.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let Some(diff) = s.pane_diff(pane).cloned() else {
        ui.label("Select one file inside a git repository.");
        return;
    };
    let state = diff.state.clone();
    let status = s.diff_view(pane).map(|v| v.status.clone());

    let anchor = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Options sit inline when the bar is wide enough, else in a hamburger menu.
                let inline = inline_width(ui);
                if ui.available_width() >= inline + MIN_LEAD {
                    inline_controls(ui, s, pane, &diff);
                } else {
                    controls(ui, s, pane, &diff);
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let tail = 150.0;
                    let text = egui::WidgetText::from(diff.rev())
                        .into_galley(
                            ui,
                            Some(egui::TextWrapMode::Extend),
                            f32::INFINITY,
                            egui::TextStyle::Button,
                        )
                        .size()
                        .x;
                    let wanted = text + ui.spacing().button_padding.x * 2.0 + 40.0;
                    let width = wanted.min(ui.available_width() - tail).max(60.0);
                    let button = ui
                        .allocate_ui(vec2(width, ui.spacing().interact_size.y), |ui| {
                            ui.add(
                                egui::Button::new(diff.rev())
                                    .truncate()
                                    .right_text("   ")
                                    .min_size(vec2(width, 0.0)),
                            )
                        })
                        .inner
                        .on_hover_text(format!(
                            "{} - choose the branch, tag or commit to compare with",
                            diff.rev()
                        ));
                    crate::pane::paint_dropdown_triangle(ui, &button);
                    if button.clicked() {
                        s.dispatch_or_notice("preview.diff.picker", &json!({"pane": pane}));
                    }
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("vs Working copy").color(tokens.git_added),
                        )
                        .truncate(),
                    );
                    button
                })
                .inner
            })
            .inner
        })
        .inner;
    if diff.picker {
        picker(ui, s, pane, &diff, anchor.rect, tokens);
    }
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
                let summary = if model.identical() {
                    format!("No changes against {}", diff.rev())
                } else {
                    format!(
                        "Change {} of {} · +{} -{}",
                        state.current.min(model.changes.len().saturating_sub(1)) + 1,
                        model.changes.len(),
                        model.added,
                        model.removed
                    )
                };
                ui.label(egui::RichText::new(summary).color(tokens.text_muted));
                let unified = state.mode == DiffMode::Unified || ui.available_width() < MIN_SPLIT;
                let target = model
                    .changes
                    .get(state.current)
                    .copied()
                    .map(|row| (state.current, row));
                let block = target.map(|(_, start)| {
                    let len = model
                        .rows
                        .iter()
                        .skip(start)
                        .take_while(|r| r.tag != Tag::Equal)
                        .count();
                    start..start + len
                });
                rows(
                    ui,
                    pane,
                    outcome.clone(),
                    unified,
                    (target, block),
                    state.jumps,
                    tokens,
                );
            }
        },
    }
}

/// Room the revision button and "vs Working copy" need before the options go inline.
const MIN_LEAD: f32 = 320.0;
const LABELS: [&str; 3] = ["Reload", "Ignore EOL", "Unified"];

/// Width of the inline options: two arrow buttons plus the labelled buttons.
fn inline_width(ui: &Ui) -> f32 {
    let pad = ui.spacing().button_padding.x * 2.0;
    let gap = ui.spacing().item_spacing.x;
    let text: f32 = LABELS
        .iter()
        .map(|t| {
            egui::WidgetText::from(*t)
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                )
                .size()
                .x
                + pad
        })
        .sum();
    text + 2.0 * 22.0 + 6.0 * gap + 8.0
}

/// The options as buttons, laid out right to left (Reload ends up rightmost).
fn inline_controls(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    diff: &fm_engine::diff_cmds::PaneDiff,
) {
    if ui.button("Reload").clicked() {
        s.dispatch_or_notice("preview.diff.reload", &json!({"pane": pane}));
    }
    if ui
        .add(egui::Button::selectable(
            diff.state.ignore_eol,
            "Ignore EOL",
        ))
        .on_hover_text("Treat CRLF and LF line endings as equal")
        .clicked()
    {
        s.dispatch_or_notice("preview.diff.eol", &json!({"pane": pane}));
    }
    if ui
        .add(egui::Button::selectable(
            diff.state.mode == DiffMode::Unified,
            "Unified",
        ))
        .on_hover_text("One column instead of side by side")
        .clicked()
    {
        s.dispatch_or_notice("preview.diff.mode", &json!({"pane": pane}));
    }
    ui.add_space(8.0);
    for (up, id, tip) in [
        (false, "preview.diff.next", "Next change"),
        (true, "preview.diff.prev", "Previous change"),
    ] {
        let (rect, resp) = ui.allocate_exact_size(vec2(22.0, 20.0), egui::Sense::click());
        let visuals = ui.style().interact(&resp);
        ui.painter()
            .rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
        let c = rect.center();
        let (w, h) = (4.5, 3.0);
        let pts = if up {
            vec![c + vec2(-w, h), c + vec2(w, h), c + vec2(0.0, -h)]
        } else {
            vec![c + vec2(-w, -h), c + vec2(w, -h), c + vec2(0.0, h)]
        };
        ui.painter().add(egui::Shape::convex_polygon(
            pts,
            visuals.fg_stroke.color,
            egui::Stroke::NONE,
        ));
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        let hover = match s.registry.get(id).and_then(|spec| spec.shortcut) {
            Some(key) => format!("{tip} ({key})"),
            None => tip.to_string(),
        };
        if resp.on_hover_text(hover).clicked() {
            s.dispatch_or_notice(id, &json!({"pane": pane}));
        }
    }
}

/// Diff options in a hamburger menu so the bar never forces the pane wider: change
/// navigation, layout, line endings and reload.
fn controls(ui: &mut Ui, s: &mut Session, pane: PaneId, diff: &fm_engine::diff_cmds::PaneDiff) {
    let button = egui::Button::new("").min_size(vec2(28.0, 0.0));
    let (response, _) = egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
        let mut run = |ui: &mut Ui, label: String, id: &str| {
            let mut button = egui::Button::new(label);
            if let Some(key) = s.registry.get(id).and_then(|spec| spec.shortcut) {
                button = button.shortcut_text(key);
            }
            if ui.add(button).clicked() {
                s.dispatch_or_notice(id, &json!({"pane": pane}));
                ui.close();
            }
        };
        run(ui, "Previous change".into(), "preview.diff.prev");
        run(ui, "Next change".into(), "preview.diff.next");
        ui.separator();
        let unified = diff.state.mode == DiffMode::Unified;
        run(
            ui,
            format!(
                "{} Unified (one column)",
                if unified { "[x]" } else { "[ ]" }
            ),
            "preview.diff.mode",
        );
        run(
            ui,
            format!(
                "{} Ignore line endings",
                if diff.state.ignore_eol { "[x]" } else { "[ ]" }
            ),
            "preview.diff.eol",
        );
        ui.separator();
        run(ui, "Reload".into(), "preview.diff.reload");
    });
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Diff options"));
    // The bundled fonts have no hamburger glyph: draw it.
    let stroke = ui.style().interact(&response).fg_stroke;
    let c = response.rect.center();
    for dy in [-4.0, 0.0, 4.0] {
        ui.painter().hline(c.x - 7.0..=c.x + 7.0, c.y + dy, stroke);
    }
    response.on_hover_text("Diff options");
}

/// Branch and tag picker under the revision button: a search box, then Recent, Local, Remote and
/// Tags, with `feature/x` style names nested into folders. Enter applies the typed text as a
/// revision, so `HEAD~3` or a commit id works too.
fn picker(
    ui: &mut Ui,
    s: &mut Session,
    pane: PaneId,
    diff: &fm_engine::diff_cmds::PaneDiff,
    below: egui::Rect,
    tokens: &Tokens,
) {
    let search_id = ui.make_persistent_id(egui::IdSalt::new(("diff-picker-q", pane)));
    let mut query: String = ui.data(|d| d.get_temp(search_id)).unwrap_or_default();
    let mut chosen: Option<String> = None;
    let mut close = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let area = egui::Area::new(egui::Id::NULL.with(("diff-picker", pane)))
        .order(egui::Order::Foreground)
        .fixed_pos(below.left_bottom() + vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                // Drag the bottom-right corner to resize; the size is remembered.
                let screen = ui.ctx().content_rect();
                let max = vec2(
                    (screen.right() - below.left() - 8.0).max(320.0),
                    (screen.bottom() - below.bottom() - 12.0).max(200.0),
                );
                egui::Resize::default()
                    .id_salt(("diff-picker-size", pane))
                    .default_size(vec2(520.0_f32.min(max.x), 460.0_f32.min(max.y)))
                    .min_size(vec2(300.0, 180.0))
                    .max_size(max)
                    .with_stroke(false)
                    .show(ui, |ui| {
                        let search = ui.add(
                            egui::TextEdit::singleline(&mut query)
                                .hint_text("Search branches, tags or type a revision")
                                .desired_width(f32::INFINITY),
                        );
                        if !search.has_focus()
                            && !ui.data(|d| {
                                d.get_temp::<bool>(search_id.with("seen")).unwrap_or(false)
                            })
                        {
                            search.request_focus();
                        }
                        ui.data_mut(|d| d.insert_temp(search_id.with("seen"), true));
                        if search.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let typed = query.trim();
                            if !typed.is_empty() {
                                chosen = Some(typed.to_string());
                            }
                        }
                        ui.add_space(4.0);
                        let q = query.trim().to_lowercase();
                        match &diff.refs {
                            None => {
                                ui.label(
                                    egui::RichText::new("Loading...").color(tokens.text_muted),
                                );
                            }
                            Some(refs) => {
                                ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        if (q.is_empty() || "head".contains(&q))
                                            && ui
                                                .selectable_label(
                                                    diff.rev() == "HEAD",
                                                    "HEAD (last commit)",
                                                )
                                                .clicked()
                                        {
                                            chosen = Some("HEAD".to_string());
                                        }
                                        let sections: [(&str, &[String]); 4] = [
                                            ("Recent", &refs.recent),
                                            ("Local", &refs.local),
                                            ("Remote", &refs.remote),
                                            ("Tags", &refs.tags),
                                        ];
                                        for (title, names) in sections {
                                            let shown: Vec<&str> = names
                                                .iter()
                                                .map(String::as_str)
                                                .filter(|n| {
                                                    q.is_empty() || n.to_lowercase().contains(&q)
                                                })
                                                .collect();
                                            if shown.is_empty() {
                                                continue;
                                            }
                                            let open = (!q.is_empty() || title == "Recent")
                                                .then_some(true);
                                            egui::CollapsingHeader::new(
                                                egui::RichText::new(title).strong(),
                                            )
                                            .id_salt(("diff-picker-sec", pane, title))
                                            .default_open(title == "Recent" || title == "Local")
                                            .open(open)
                                            .show(
                                                ui,
                                                |ui| {
                                                    names_tree(
                                                        ui,
                                                        pane,
                                                        title,
                                                        &shown,
                                                        refs.current.as_deref(),
                                                        diff.rev(),
                                                        tokens,
                                                        &mut chosen,
                                                    );
                                                },
                                            );
                                        }
                                        if refs.truncated {
                                            ui.label(
                                                egui::RichText::new(
                                                    "List truncated; type a name and press Enter.",
                                                )
                                                .color(tokens.text_muted),
                                            );
                                        }
                                    });
                            }
                        }
                    });
            });
        });
    ui.data_mut(|d| d.insert_temp(search_id, query));
    let outside = ui.input(|i| i.pointer.any_pressed())
        && ui
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|p| !area.response.rect.contains(p) && !below.contains(p));
    close |= outside;
    if let Some(rev) = chosen {
        s.dispatch_or_notice("preview.diff.rev", &json!({"pane": pane, "rev": rev}));
        close = true;
    }
    if close {
        ui.data_mut(|d| {
            d.remove::<bool>(search_id.with("seen"));
            d.remove::<String>(search_id);
        });
        if s.pane_diff(pane).is_some_and(|d| d.picker) {
            s.dispatch_or_notice("preview.diff.picker", &json!({"pane": pane, "open": false}));
        }
    }
}

/// Names grouped by their first `/` segment into collapsible folders (one level, like branch
/// prefixes `feature/` and `bugfix/`); the rest are leaves.
#[allow(clippy::too_many_arguments)]
fn names_tree(
    ui: &mut Ui,
    pane: PaneId,
    section: &str,
    names: &[&str],
    current: Option<&str>,
    selected: &str,
    tokens: &Tokens,
    chosen: &mut Option<String>,
) {
    let mut folders: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    let mut leaves: Vec<&str> = Vec::new();
    for name in names {
        match name.split_once('/') {
            Some((head, _)) if section != "Remote" || name.matches('/').count() > 1 => {
                folders.entry(head).or_default().push(name);
            }
            _ => leaves.push(name),
        }
    }
    let leaf = |ui: &mut Ui, name: &str, chosen: &mut Option<String>| {
        let star = current == Some(name);
        let text = if star {
            format!("* {name}")
        } else {
            name.to_string()
        };
        let label = if star {
            egui::RichText::new(text).color(tokens.accent)
        } else {
            egui::RichText::new(text)
        };
        if ui.selectable_label(selected == name, label).clicked() {
            *chosen = Some(name.to_string());
        }
    };
    for name in &leaves {
        leaf(ui, name, chosen);
    }
    for (folder, members) in folders {
        egui::CollapsingHeader::new(folder)
            .id_salt(("diff-picker-folder", pane, section, folder))
            .default_open(false)
            .show(ui, |ui| {
                for name in members {
                    leaf(ui, name, chosen);
                }
            });
    }
}

#[derive(Clone)]
struct HorizontalExtent {
    document: std::sync::Arc<Outcome>,
    font: FontId,
    width: f32,
}

fn rows(
    ui: &mut Ui,
    pane: PaneId,
    document: std::sync::Arc<Outcome>,
    unified: bool,
    (target, block): (Option<(usize, usize)>, Option<std::ops::Range<usize>>),
    jumps: u64,
    tokens: &Tokens,
) {
    let Outcome::Text(model) = document.as_ref() else {
        return;
    };
    let rows = &model.rows;
    let font = FontId::monospace(tokens.font_size);
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + 2.0;
    let extent_id = ui.make_persistent_id(egui::IdSalt::new(("diff-width", pane, unified)));
    let mut extent: HorizontalExtent = ui
        .ctx()
        .data(|d| d.get_temp(extent_id))
        .filter(|extent: &HorizontalExtent| {
            std::sync::Arc::ptr_eq(&extent.document, &document) && extent.font == font
        })
        .unwrap_or_else(|| HorizontalExtent {
            document: document.clone(),
            font: font.clone(),
            width: 0.0,
        });
    let seen = ui.make_persistent_id(egui::IdSalt::new(("diff-current", pane)));
    let last: Option<(usize, u64)> = ui.ctx().data(|d| d.get_temp(seen));
    let mut area = ScrollArea::both()
        .id_salt(("diff-scroll", pane))
        .auto_shrink(false);
    if let Some((index, row)) = target
        && last != Some((index, jumps))
    {
        let visual = if unified {
            unified_index(rows, row)
        } else {
            row
        };
        // The change's first line lands at the top of the view.
        area = area.vertical_scroll_offset(visual as f32 * row_h);
    }
    ui.ctx().data_mut(|d| match target {
        Some((i, _)) => {
            d.insert_temp(seen, (i, jumps));
        }
        None => d.remove::<(usize, u64)>(seen),
    });

    let total = if unified {
        unified_index(rows, rows.len())
    } else {
        rows.len()
    };
    let out = area.show_viewport(ui, |ui, viewport| {
        let origin = ui.min_rect().min;
        let width = viewport.width();
        let half = ((width - 4.0) / 2.0).max(0.0);
        let gutter = if unified { NUM_W + 16.0 } else { NUM_W };
        let text_width = (if unified { width } else { half } - gutter).max(0.0);
        let first = ((viewport.top() / row_h).floor().max(0.0) as usize).min(total);
        let end = ((viewport.bottom() / row_h).ceil().max(0.0) as usize + 1).min(total);
        let line_rect = |index: usize| {
            egui::Rect::from_min_size(
                origin + vec2(viewport.left(), index as f32 * row_h),
                vec2(width, row_h),
            )
        };
        let mut measured = extent.width;
        if unified {
            let mut at = 0;
            for (index, row) in rows.iter().enumerate() {
                let n = unified_len(row);
                if at + n > first && at < end {
                    let current = block.as_ref().is_some_and(|b| b.contains(&index));
                    measured = measured.max(unified_row(
                        ui,
                        row,
                        current,
                        line_rect(at),
                        viewport.left(),
                        &font,
                        tokens,
                    ));
                }
                at += n;
                if at >= end {
                    break;
                }
            }
        } else {
            for (offset, row) in rows.get(first..end).unwrap_or_default().iter().enumerate() {
                let line = line_rect(first + offset);
                let left = egui::Rect::from_min_size(line.min, vec2(half, row_h));
                let right =
                    egui::Rect::from_min_size(line.min + vec2(half + 4.0, 0.0), vec2(half, row_h));
                for (right, rect) in [(false, left), (true, right)] {
                    measured =
                        measured.max(side(ui, row, right, rect, viewport.left(), &font, tokens));
                }
                if block
                    .as_ref()
                    .is_some_and(|b| b.contains(&(first + offset)))
                {
                    current_bar(ui, line, tokens);
                }
            }
        }
        extent.width = measured;
        ui.set_min_size(vec2(
            width + (measured - text_width).max(0.0),
            total as f32 * row_h,
        ));
    });
    ui.ctx().data_mut(|d| d.insert_temp(extent_id, extent));
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::NULL.with("diff-offset-test"),
            out.state.unclamped_offset().y,
        );
        d.insert_temp(
            egui::Id::NULL.with("diff-scroll-test"),
            (out.state.unclamped_offset(), out.inner_rect),
        );
    });
    let _ = &out;
}

/// Accent bar at the left edge of the change that Next/Previous landed on.
fn current_bar(ui: &Ui, rect: egui::Rect, tokens: &Tokens) {
    let bar = egui::Rect::from_min_size(rect.min, vec2(3.0, rect.height()));
    ui.painter().rect_filled(bar, 0.0, tokens.accent);
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
    ui: &Ui,
    row: &Row,
    right: bool,
    rect: egui::Rect,
    x: f32,
    font: &FontId,
    tokens: &Tokens,
) -> f32 {
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
            let width = galley.size().x;
            let text_rect =
                egui::Rect::from_min_max(rect.min + vec2(NUM_W.min(rect.width()), 0.0), rect.max);
            painter
                .with_clip_rect(painter.clip_rect().intersect(text_rect))
                .galley(rect.left_top() + vec2(NUM_W - x, 1.0), galley, tokens.text);
            width
        }
        None => {
            painter.rect_filled(rect, 0.0, tokens.diff_del_bg.gamma_multiply(0.3));
            0.0
        }
    }
}

fn unified_row(
    ui: &Ui,
    row: &Row,
    current: bool,
    rect: egui::Rect,
    x: f32,
    font: &FontId,
    tokens: &Tokens,
) -> f32 {
    let one = |right: bool, rect: egui::Rect| {
        let line = if right { &row.right } else { &row.left };
        let Some(l) = line else { return 0.0 };
        let (bg, word) = backgrounds(row.tag, right, tokens);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, bg);
        if current {
            current_bar(ui, rect, tokens);
        }
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
        let width = galley.size().x;
        let text_rect = egui::Rect::from_min_max(
            rect.min + vec2((NUM_W + 16.0).min(rect.width()), 0.0),
            rect.max,
        );
        painter
            .with_clip_rect(painter.clip_rect().intersect(text_rect))
            .galley(
                rect.left_top() + vec2(NUM_W + 16.0 - x, 1.0),
                galley,
                tokens.text,
            );
        width
    };
    match row.tag {
        Tag::Equal | Tag::Insert => one(row.tag == Tag::Insert, rect),
        Tag::Delete => one(false, rect),
        Tag::Replace => one(false, rect).max(one(true, rect.translate(vec2(0.0, rect.height())))),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use std::time::{Duration, Instant};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    }

    fn rig(changes: &[usize]) -> (tempfile::TempDir, Session, PaneId) {
        let base: Vec<String> = (0..300).map(|i| format!("line {i}")).collect();
        rig_with_base(changes, base)
    }

    fn rig_with_base(changes: &[usize], base: Vec<String>) -> (tempfile::TempDir, Session, PaneId) {
        let dir = tempfile::tempdir().expect("tmp");
        let mut changed = base.clone();
        for i in changes {
            changed[*i] = format!("CHANGED {i}");
        }
        let file = dir.path().join("a.txt");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(&file, base.join("\n")).expect("a");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "i"]);
        std::fs::write(&file, changed.join("\n")).expect("b");
        let mut s = Session::new().expect("session");
        let folder = fm_vfs::VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
        let files = s.layout.focused();
        s.layout
            .state_mut(files)
            .expect("pane")
            .navigate(folder.clone());
        s.ensure_read(folder).expect("read");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.pane_view_ready(files) {
            assert!(Instant::now() < deadline);
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let uri = fm_vfs::VfsUri::parse(&file.to_string_lossy()).expect("uri");
        s.dispatch("preview.open", &serde_json::json!({"uri": uri.to_string()}))
            .expect("open");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| s.pane_kind(*p) == Some(fm_layout::PaneKind::Preview))
            .expect("preview");
        while !(s.pane_diff(pane).is_some_and(|d| d.active)
            && matches!(
                s.diff_view(pane).map(|v| &v.status),
                Some(DiffStatus::Ready(_))
            ))
        {
            assert!(Instant::now() < deadline, "diff never became ready");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        (dir, s, pane)
    }

    fn harness(s: Session, pane: PaneId) -> Harness<'static, Session> {
        let tokens = Tokens::dark();
        Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    show(ui, s, pane, &tokens);
                },
                s,
            )
    }

    fn offset(h: &Harness<Session>) -> f32 {
        h.ctx
            .data(|d| d.get_temp::<f32>(egui::Id::NULL.with("diff-offset-test")))
            .unwrap_or(-1.0)
    }

    fn step(h: &mut Harness<Session>, id: &str) {
        h.state_mut()
            .dispatch(id, &serde_json::json!({}))
            .expect("dispatch");
        h.run();
        h.run();
    }

    #[test]
    fn next_and_previous_scroll_to_the_changed_blocks() {
        let (_dir, s, pane) = rig(&[10, 150, 280]);
        let mut harness = harness(s, pane);
        harness.run();
        let start = offset(&harness);
        step(&mut harness, "preview.diff.next");
        let second = offset(&harness);
        assert!(second > start + 500.0, "Next: {start} -> {second}");
        step(&mut harness, "preview.diff.next");
        let third = offset(&harness);
        assert!(third > second + 500.0, "Next: {second} -> {third}");
        step(&mut harness, "preview.diff.prev");
        assert!(offset(&harness) < third - 500.0, "Prev did not scroll back");
    }

    #[test]
    fn a_single_change_is_scrolled_back_to() {
        let (_dir, s, pane) = rig(&[200]);
        let mut harness = harness(s, pane);
        harness.run();
        let at_change = offset(&harness);
        assert!(at_change > 500.0, "first change not shown: {at_change}");
    }

    fn positions(h: &Harness<Session>, value: &str) -> Vec<egui::Pos2> {
        fn collect(shape: &egui::Shape, value: &str, out: &mut Vec<egui::Pos2>) {
            match shape {
                egui::Shape::Text(text) if text.galley.text() == value => out.push(text.pos),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, value, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for shape in &h.output().shapes {
            collect(&shape.shape, value, &mut out);
        }
        out
    }

    fn horizontal(h: &Harness<Session>) -> (egui::Vec2, egui::Rect) {
        h.ctx
            .data(|d| d.get_temp(egui::Id::NULL.with("diff-scroll-test")))
            .expect("scroll")
    }

    #[test]
    fn split_horizontal_scroll_links_both_sides_and_keeps_gutters_fixed_on_resize() {
        let base: Vec<_> = (0..300)
            .map(|i| format!("line {i} {}", "wide text ".repeat(60)))
            .collect();
        let text = base[11].clone();
        let (_dir, s, pane) = rig_with_base(&[10], base);
        let mut h = harness(s, pane);
        h.run();
        let before = positions(&h, &text);
        let gutters = positions(&h, "12");
        assert_eq!(before.len(), 2);
        assert_eq!(gutters.len(), 2);
        let (_, viewport) = horizontal(&h);
        for fraction in [0.25, 0.75] {
            h.hover_at(egui::pos2(
                viewport.left() + viewport.width() * fraction,
                viewport.center().y,
            ));
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(-120.0, 0.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            });
            h.run();
        }
        let after = positions(&h, &text);
        assert_eq!(after.len(), 2);
        assert!(horizontal(&h).0.x > 100.0);
        assert!((before[0].x - after[0].x - (before[1].x - after[1].x)).abs() < 0.1);
        assert!((before[0].x - after[0].x) > 100.0);
        for (before, after) in gutters.iter().zip(positions(&h, "12")) {
            assert!(before.distance(after) < 0.5, "{before:?} -> {after:?}");
        }
        h.set_size(vec2(620.0, 400.0));
        h.run();
        let (_, smaller) = horizontal(&h);
        assert!(
            smaller.width() < viewport.width() - 150.0,
            "{viewport:?} -> {smaller:?}"
        );
        assert!(
            horizontal(&h).0.x > 100.0,
            "resize retains the shared offset"
        );
        h.state_mut()
            .dispatch("preview.diff.mode", &serde_json::json!({}))
            .expect("unified");
        h.run();
        assert_eq!(positions(&h, &text).len(), 1);
        step(&mut h, "preview.diff.next");
        assert!(offset(&h) >= 0.0);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod folder_tests {
    use super::*;
    use egui_kittest::Harness;
    use std::time::{Duration, Instant};

    fn git(dir: &std::path::Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    }

    fn pump(s: &mut Session, ms: u64) {
        let end = Instant::now() + Duration::from_millis(ms);
        while Instant::now() < end {
            s.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Switching from a file whose diff is showing to a folder must not leave the pane loading.
    #[test]
    fn a_folder_after_a_diffed_file_settles() {
        let tmp = tempfile::tempdir().expect("tmp");
        let d = tmp.path();
        git(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir(d.join("sub")).expect("dir");
        std::fs::write(d.join("sub/a.txt"), "x\n").expect("file");
        std::fs::write(d.join("f.txt"), "1\n").expect("f");
        git(d, &["add", "."]);
        git(d, &["commit", "-q", "-m", "i"]);
        std::fs::write(d.join("f.txt"), "2\n").expect("f");
        let mut s = Session::new().expect("session");
        let dir = fm_vfs::VfsUri::parse(&d.to_string_lossy()).expect("uri");
        let files = s.layout.focused();
        s.layout
            .state_mut(files)
            .expect("pane")
            .navigate(dir.clone());
        s.ensure_read(dir).expect("read");
        pump(&mut s, 300);
        let file = fm_vfs::VfsUri::parse(&d.join("f.txt").to_string_lossy()).expect("uri");
        s.dispatch(
            "preview.open",
            &serde_json::json!({"uri": file.to_string()}),
        )
        .expect("open");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| s.pane_kind(*p) == Some(fm_layout::PaneKind::Preview))
            .expect("preview");
        pump(&mut s, 500);
        assert!(s.pane_diff(pane).is_some_and(|d| d.active));
        let tokens = Tokens::dark();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 400.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    crate::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        harness.run();
        let sub = fm_vfs::VfsUri::parse(&d.join("sub").to_string_lossy()).expect("uri");
        harness
            .state_mut()
            .dispatch("preview.open", &serde_json::json!({"uri": sub.to_string()}))
            .expect("open folder");
        for _ in 0..30 {
            harness.state_mut().poll();
            harness.run();
            std::thread::sleep(Duration::from_millis(10));
        }
        let s = harness.state();
        assert!(s.pane_diff(pane).is_none());
        assert!(
            s.preview_slot(pane)
                .is_some_and(|sl| matches!(sl.body, fm_engine::preview_cmds::Body::Static(_))),
            "folder preview stuck"
        );
    }
}
