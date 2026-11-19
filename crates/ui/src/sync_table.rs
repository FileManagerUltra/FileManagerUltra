//! The comparison grid of the Synchronize Folders window: a virtualized tree of rows with a
//! per-row action menu, the action filter and the detail strip. It only reads the plan and
//! queues `sync.*` commands.

use egui::{Color32, Id, RichText};
use fm_engine::item_cmds::display_path;
use fm_engine::sync_hub::{Phase, RowFilter, SortKey, SyncState};
use fm_sync::{Action, Item, NodeKind, Side, Status, SyncMode};
use fm_vfs::VfsUri;
use serde_json::json;

use crate::sync::{Command, action_hint, choices, look, node_text};
use crate::sync_icons::{self, Icon};
use crate::theme::Tokens;

/// The grid, leaving `reserve` points below it for what the caller draws next. With
/// `folders_as_header` the two side headers name the folders (and source/target) being compared.
pub(crate) fn table(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    commands: &mut Vec<Command>,
    reserve: f32,
    folders_as_header: bool,
) {
    let row_h = tokens.row_height;
    let Some(plan) = &state.plan else {
        ui.add_space(24.0);
        ui.vertical_centered(|ui| {
            let msg = match &state.phase {
                Phase::Comparing { left, right } => {
                    format!("Scanning… {left} items on the left, {right} on the right")
                }
                Phase::Failed(e) => e.clone(),
                _ if state.awaiting_login => {
                    "Sign in to the server; the comparison starts once you are connected."
                        .to_string()
                }
                _ => "Choose two folders and press Compare.".to_string(),
            };
            ui.label(RichText::new(msg).color(tokens.text_muted));
        });
        ui.add_space(24.0);
        return;
    };
    let (mode, source) = (state.profile.mode, state.profile.source);
    let width = ui.available_width();
    let side = ((width - 90.0) / 2.0).max(120.0);
    let (name_w, size_w, time_w) = (side * 0.5, side * 0.2, side * 0.3);
    let marker_w = 90.0;
    let tree = state.sort.is_tree();
    let (bar, _) = ui.allocate_exact_size(egui::vec2(width, row_h + 2.0), egui::Sense::hover());
    ui.painter()
        .rect_filled(bar, tokens.corner_radius, tokens.hover_bg);
    let mut x = bar.left();
    let side_title = |left: bool| -> String {
        if !folders_as_header {
            return if left { "Left" } else { "Right" }.to_string();
        }
        let root = if left {
            &plan.left_root
        } else {
            &plan.right_root
        };
        let role = match (mode, source == Side::Left) {
            (SyncMode::TwoWay, _) => "",
            (_, from_left) if from_left == left => "Source: ",
            _ => "Target: ",
        };
        let path = display_path(root);
        let room = ((side - 24.0) / (tokens.font_size * 0.55)) as usize;
        let room = room.saturating_sub(role.len()).max(8);
        let count = path.chars().count();
        let tail: String = if count > room {
            let keep: String = path.chars().skip(count - (room - 1)).collect();
            format!("…{keep}")
        } else {
            path
        };
        format!("{role}{tail}")
    };
    let columns = [
        (side_title(true), name_w, Some(SortKey::Name), false),
        ("Size".to_string(), size_w, Some(SortKey::Size), true),
        (
            "Modified".to_string(),
            time_w,
            Some(SortKey::Modified),
            false,
        ),
        ("Action".to_string(), marker_w, Some(SortKey::Action), false),
        (side_title(false), name_w, Some(SortKey::Name), false),
        ("Size".to_string(), size_w, Some(SortKey::Size), true),
        (
            "Modified".to_string(),
            time_w,
            Some(SortKey::Modified),
            false,
        ),
    ];
    for (n, (text, w, key, right)) in columns.into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(egui::pos2(x, bar.top()), egui::vec2(w, bar.height()));
        x += w;
        let Some(key) = key else { continue };
        let response = ui.interact(
            rect,
            Id::NULL.with(("fm-sync-head", n)),
            egui::Sense::click(),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Sort by {text}"))
        });
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, tokens.corner_radius, tokens.selection_bg);
        }
        let active = state.sort == key;
        let color = if active {
            tokens.text
        } else {
            tokens.text_muted
        };
        let font = egui::FontId::proportional(tokens.font_size);
        let (pos, anchor) = if right {
            (
                egui::pos2(rect.right() - 14.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
            )
        } else if key == SortKey::Action {
            (
                rect.center() - egui::vec2(14.0, 0.0),
                egui::Align2::CENTER_CENTER,
            )
        } else {
            (
                egui::pos2(rect.left() + 4.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
            )
        };
        let galley_rect = ui
            .painter()
            .with_clip_rect(rect)
            .text(pos, anchor, &text, font, color);
        if active {
            // A small triangle: up is ascending.
            let c = egui::pos2(galley_rect.right() + 8.0, rect.center().y);
            let up = !state.descending;
            let (a, b, d) = if up {
                (
                    c + egui::vec2(-4.0, 2.5),
                    c + egui::vec2(4.0, 2.5),
                    c + egui::vec2(0.0, -3.0),
                )
            } else {
                (
                    c + egui::vec2(-4.0, -2.5),
                    c + egui::vec2(4.0, -2.5),
                    c + egui::vec2(0.0, 3.0),
                )
            };
            ui.painter().add(egui::Shape::convex_polygon(
                vec![a, b, d],
                color,
                egui::Stroke::NONE,
            ));
        }
        if key == SortKey::Action {
            action_filter_menu(ui, tokens, state, rect, commands);
        }
        let next = if !active {
            (key, false, "Sort ascending")
        } else if !state.descending {
            (key, true, "Sort descending")
        } else {
            (SortKey::Path, false, "Back to folder order")
        };
        if response.on_hover_text(next.2).clicked() {
            let name = match next.0 {
                SortKey::Path => "path",
                SortKey::Name => "name",
                SortKey::Size => "size",
                SortKey::Modified => "modified",
                SortKey::Action => "action",
            };
            commands.push(("sync.sort", json!({ "key": name, "descending": next.1 })));
        }
    }
    if state.rows.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            let msg = if plan.items.is_empty() {
                "Both folders are empty."
            } else if state.filter == RowFilter::All {
                "Nothing to show."
            } else {
                "No rows match this filter."
            };
            ui.label(RichText::new(msg).color(tokens.text_muted));
        });
    }
    let view_id = Id::NULL.with(("fm-sync-tree", &state.profile.id));
    let mut view: TreeView = ui.data_mut(|d| d.get_temp(view_id)).unwrap_or_default();
    let shown = visible_rows(state, tree.then_some(&view.collapsed));
    let selected_item = view
        .selected
        .and_then(|i| plan.items.get(i as usize))
        .filter(|i| tree || !i.rel.is_empty());
    // The path details under the table: measured once drawn; until then estimated from the
    // font and spacing (three single lines in a frame), so the list leaves room for them and
    // a click never makes the window grow (and, centred, move).
    let detail_id = view_id.with("detail-height");
    let detail_h = if selected_item.is_some() {
        ui.data(|d| d.get_temp::<f32>(detail_id))
            .unwrap_or_else(|| {
                let line = ui.text_style_height(&egui::TextStyle::Body);
                let gap = ui.spacing().item_spacing.y;
                3.0 * line + 3.0 * gap + 2.0 * DETAIL_MARGIN + 4.0
            })
    } else {
        0.0
    };
    // The list takes the room the page gives it, but never less than two rows. When the
    // details would not fit under it as well (Advanced with the settings open on a small
    // screen) they wait for room instead of making the page, and with it the window, taller.
    let room = ui.available_height() - reserve;
    let list_min = LIST_MIN_ROWS * row_h;
    let selected_item = selected_item.filter(|_| room - detail_h >= list_min);
    let detail_h = if selected_item.is_some() {
        detail_h
    } else {
        0.0
    };
    let mut toggled: Option<String> = None;
    let mut picked: Option<u32> = None;
    // show_rows spaces rows by the outer item spacing; rows are drawn flush, so it must be zero.
    let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.y, 0.0);
    let out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
        .max_height((room - detail_h).max(list_min))
        // egui's default (64) would overrule `list_min` and push the page past its room.
        .min_scrolled_height(list_min)
        .show_rows(ui, row_h, shown.len(), |ui, range| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            for n in range {
                let Some((index, item)) = shown.get(n).and_then(|i| {
                    plan.items
                        .get(usize::try_from(*i).unwrap_or(usize::MAX))
                        .map(|item| (*i, item))
                }) else {
                    continue;
                };
                let is_dir = tree && item.kind() == NodeKind::Dir;
                let shape = RowShape {
                    policy: (mode, source),
                    tree,
                    collapsed: is_dir && view.collapsed.contains(&item.rel),
                    rollup: is_dir.then(|| rollup(plan, index as usize)),
                    selected: view.selected == Some(index),
                    left_root: &plan.left_root,
                    right_root: &plan.right_root,
                };
                let event = row(
                    ui,
                    tokens,
                    item,
                    &shape,
                    row_h,
                    [name_w, size_w, time_w, marker_w],
                    commands,
                );
                if event.toggle {
                    toggled = Some(item.rel.clone());
                }
                if event.select {
                    picked = Some(index);
                }
            }
        });
    ui.spacing_mut().item_spacing.y = spacing;
    if let Some(rel) = toggled
        && !view.collapsed.remove(&rel)
    {
        view.collapsed.insert(rel);
    }
    if let Some(i) = picked {
        view.selected = if view.selected == Some(i) {
            None
        } else {
            Some(i)
        };
    }
    ui.data_mut(|d| d.insert_temp(view_id, view));
    // Outline and column dividers make the list read as a scrollable table.
    let table = egui::Rect::from_min_max(bar.left_top(), out.inner_rect.right_bottom());
    let line = egui::Stroke::new(1.0, tokens.text_muted.gamma_multiply(0.45));
    let painter = ui.painter();
    let mut x = bar.left();
    for w in [name_w, size_w, time_w, marker_w, name_w, size_w] {
        x += w;
        painter.vline(
            x,
            table.y_range(),
            egui::Stroke::new(1.0, line.color.gamma_multiply(0.5)),
        );
    }
    painter.rect_stroke(table, tokens.corner_radius, line, egui::StrokeKind::Inside);
    if let Some(item) = selected_item {
        let top = ui.cursor().top();
        detail_strip(ui, tokens, plan, item);
        let used = (ui.cursor().top() - top).ceil();
        if ui.data(|d| d.get_temp::<f32>(detail_id)) != Some(used) {
            ui.data_mut(|d| d.insert_temp(detail_id, used));
            ui.ctx().request_repaint();
        }
    }
}

const DETAIL_MARGIN: f32 = 6.0;
/// The lowest the list's scroll area gets, in rows.
const LIST_MIN_ROWS: f32 = 2.0;

/// Folders the user has folded shut and the row whose details are shown below the table.
#[derive(Clone, Default)]
struct TreeView {
    collapsed: std::collections::HashSet<String>,
    selected: Option<u32>,
}

/// What `row` needs to know beyond the item itself.
struct RowShape<'a> {
    policy: (SyncMode, Side),
    tree: bool,
    collapsed: bool,
    rollup: Option<String>,
    selected: bool,
    left_root: &'a VfsUri,
    right_root: &'a VfsUri,
}

/// How a cell's text is placed; names are shortened in the middle when they do not fit.
#[derive(Clone, Copy, PartialEq)]
enum Align {
    Name,
    Left,
    Right,
}

#[derive(Default)]
struct RowEvent {
    toggle: bool,
    select: bool,
}

/// Plan indices to list: the filtered rows without the contents of folded folders.
/// A funnel at the right of the Action header: show only rows that will do one kind of thing.
fn action_filter_menu(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    state: &SyncState,
    header: egui::Rect,
    commands: &mut Vec<Command>,
) {
    let rect = egui::Rect::from_center_size(
        egui::pos2(header.right() - 13.0, header.center().y),
        egui::vec2(22.0, header.height()),
    );
    let response = ui.interact(
        rect,
        Id::NULL.with("fm-sync-action-filter"),
        egui::Sense::click(),
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Filter by action"));
    let on = state.action_filter.is_some();
    if on || response.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(1.0, 3.0)),
            tokens.corner_radius,
            tokens.selection_bg,
        );
    }
    let color = if on { tokens.accent } else { tokens.text_muted };
    sync_icons::paint(ui.painter(), Icon::Funnel, rect.center(), 13.0, color);
    let response = response
        .on_hover_text("Filter by action")
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&response).show(|ui| {
        let options = [
            (None, "all", "All actions"),
            (
                Some(Action::CopyToRight),
                "copy_to_right",
                "Copy to the right",
            ),
            (Some(Action::CopyToLeft), "copy_to_left", "Copy to the left"),
            (
                Some(Action::DeleteLeft),
                "delete_left",
                "Delete on the left",
            ),
            (
                Some(Action::DeleteRight),
                "delete_right",
                "Delete on the right",
            ),
            (Some(Action::Skip), "skip", "Skipped"),
            (Some(Action::None), "none", "No action"),
        ];
        for (action, name, label) in options {
            let icon = if name == "all" {
                Icon::Funnel
            } else {
                Icon::for_action(name)
            };
            if sync_icons::menu_item(ui, tokens, icon, label, state.action_filter == action)
                .clicked()
            {
                commands.push(("sync.filter_action", json!({ "action": name })));
            }
        }
    });
}

fn visible_rows(
    state: &SyncState,
    collapsed: Option<&std::collections::HashSet<String>>,
) -> Vec<u32> {
    let (Some(plan), Some(collapsed)) = (&state.plan, collapsed) else {
        return state.rows.clone();
    };
    if collapsed.is_empty() {
        return state.rows.clone();
    }
    let prefixes: Vec<String> = collapsed.iter().map(|rel| format!("{rel}/")).collect();
    state
        .rows
        .iter()
        .copied()
        .filter(|&i| {
            plan.items
                .get(i as usize)
                .is_some_and(|item| !prefixes.iter().any(|p| item.rel.starts_with(p.as_str())))
        })
        .collect()
}

/// "812 files · 398 to the right · 2 conflicts" for the folder at `index` and everything in it.
fn rollup(plan: &fm_sync::Plan, index: usize) -> String {
    let Some(dir) = plan.items.get(index) else {
        return String::new();
    };
    let prefix = format!("{}/", dir.rel);
    let (mut files, mut right, mut left, mut deleted, mut conflicts) = (0, 0, 0, 0, 0);
    // Sorted by path, so everything under a folder is one contiguous run.
    let start = plan
        .items
        .partition_point(|i| i.rel.as_str() < prefix.as_str());
    for item in plan.items.iter().skip(start) {
        if !item.rel.starts_with(&prefix) {
            break;
        }
        if item.kind() == NodeKind::Dir {
            continue;
        }
        files += 1;
        match item.action {
            Action::CopyToRight => right += 1,
            Action::CopyToLeft => left += 1,
            Action::DeleteLeft | Action::DeleteRight => deleted += 1,
            _ => {}
        }
        if matches!(item.status, Status::Conflict | Status::Unknown) {
            conflicts += 1;
        }
    }
    let mut parts = vec![format!("{files} file{}", if files == 1 { "" } else { "s" })];
    for (n, what) in [
        (right, "to the right"),
        (left, "to the left"),
        (deleted, "deleted"),
        (conflicts, "in conflict"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    parts.join("  ·  ")
}

/// Full name and both full paths of the selected row, wrapped so nothing is cut off.
fn detail_strip(ui: &mut egui::Ui, tokens: &Tokens, plan: &fm_sync::Plan, item: &Item) {
    let full = |root: &VfsUri, present: bool| full_path(root, item, present);
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.0,
            tokens.text_muted.gamma_multiply(0.45),
        ))
        .corner_radius(tokens.corner_radius)
        .inner_margin(DETAIL_MARGIN)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Single lines (full text on hover) keep the strip's height fixed.
            ui.add(egui::Label::new(RichText::new(&item.rel).color(tokens.text)).truncate());
            let side = |name: &str, text: String| {
                RichText::new(format!("{name} {text}")).color(tokens.text_muted)
            };
            ui.add(
                egui::Label::new(side("Left:", full(&plan.left_root, item.left.is_some())))
                    .truncate(),
            );
            ui.add(
                egui::Label::new(side("Right:", full(&plan.right_root, item.right.is_some())))
                    .truncate(),
            );
        });
}

/// `text` shortened in the middle so both its start and its extension stay visible.
fn fit_middle(
    painter: &egui::Painter,
    text: &str,
    font: &egui::FontId,
    color: Color32,
    max_w: f32,
) -> std::sync::Arc<egui::Galley> {
    let whole = painter.layout_no_wrap(text.to_string(), font.clone(), color);
    if whole.size().x <= max_w {
        return whole;
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (1usize, chars.len());
    let mut best = painter.layout_no_wrap("…".to_string(), font.clone(), color);
    while lo < hi {
        let keep = (lo + hi).div_ceil(2);
        let tail = (keep * 2 / 5).min(keep.saturating_sub(1)).max(1);
        let head = keep - tail;
        let cut: String = chars
            .iter()
            .take(head)
            .chain(['…'].iter())
            .chain(chars.iter().skip(chars.len().saturating_sub(tail)))
            .collect();
        let galley = painter.layout_no_wrap(cut, font.clone(), color);
        if galley.size().x <= max_w {
            best = galley;
            lo = keep;
        } else {
            hi = keep - 1;
        }
    }
    best
}

fn row(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    item: &Item,
    shape: &RowShape,
    row_h: f32,
    widths: [f32; 4],
    commands: &mut Vec<Command>,
) -> RowEvent {
    let [name_w, size_w, time_w, marker_w] = widths;
    let tree = shape.tree;
    let is_dir = tree && item.kind() == NodeKind::Dir;
    let mut event = RowEvent::default();
    let (rect, row_response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), row_h),
        egui::Sense::click(),
    );
    if is_dir {
        ui.painter()
            .rect_filled(rect, 0.0, tokens.panel.lerp_to_gamma(tokens.text, 0.1));
    }
    if row_response.hovered() || shape.selected {
        ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
    }
    if shape.selected {
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(1.0, tokens.accent),
            egui::StrokeKind::Inside,
        );
    }
    let changing = matches!(
        item.action,
        Action::CopyToRight | Action::CopyToLeft | Action::DeleteLeft | Action::DeleteRight
    );
    let deleting = matches!(item.action, Action::DeleteLeft | Action::DeleteRight);
    let equal = item.status == Status::Equal;
    if matches!(item.status, Status::Conflict | Status::Unknown) {
        ui.painter().rect_filled(rect, 0.0, tokens.sync_conflict_bg);
    } else if changing && !is_dir {
        let tint = if deleting {
            tokens.diff_del_bg
        } else {
            tokens.diff_add_bg
        };
        ui.painter().rect_filled(rect, 0.0, tint);
    }
    if changing && !is_dir {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(3.0, row_h));
        let (_, _, color) = look(item, tokens);
        ui.painter().rect_filled(bar, 0.0, color);
    }
    let font = egui::FontId::proportional(tokens.font_size);
    let indent = if tree {
        4.0 + (item.depth() as f32 * 16.0).min(name_w * 0.5)
    } else {
        0.0
    };
    let leaf = if tree {
        item.rel.rsplit('/').next().unwrap_or(&item.rel)
    } else {
        item.rel.as_str()
    };
    let mut x = rect.left();
    let painter = ui.painter().clone();
    let cell = |x: f32, text: &str, w: f32, color: Color32, pad: f32, align: Align| {
        let cell = egui::Rect::from_min_size(
            egui::pos2(x + pad, rect.top()),
            egui::vec2((w - pad - 4.0).max(0.0), row_h),
        );
        let galley = if align != Align::Name {
            painter.layout_no_wrap(text.to_string(), font.clone(), color)
        } else {
            fit_middle(&painter, text, &font, color, cell.width())
        };
        let left = if align == Align::Right {
            (cell.right() - galley.size().x - 10.0).max(cell.left())
        } else {
            cell.left()
        };
        painter.with_clip_rect(cell).galley(
            egui::pos2(left, cell.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    };
    for s in [Side::Left, Side::Right] {
        let (size, when) = node_text(item, s);
        let present = item.node(s).is_some();
        let name = if present {
            if item.kind() == NodeKind::Dir {
                format!("{leaf}/")
            } else {
                leaf.to_string()
            }
        } else {
            String::new()
        };
        let name_color = if equal {
            tokens.text_muted
        } else {
            tokens.text
        };
        if is_dir && s == Side::Left {
            let c = egui::pos2(rect.left() + indent + 6.0, rect.center().y);
            let pts = if shape.collapsed {
                vec![
                    c + egui::vec2(-2.5, -4.0),
                    c + egui::vec2(-2.5, 4.0),
                    c + egui::vec2(3.0, 0.0),
                ]
            } else {
                vec![
                    c + egui::vec2(-4.0, -2.5),
                    c + egui::vec2(4.0, -2.5),
                    c + egui::vec2(0.0, 3.0),
                ]
            };
            painter.add(egui::Shape::convex_polygon(
                pts,
                tokens.text,
                egui::Stroke::NONE,
            ));
            cell(x, &name, name_w, tokens.text, indent + 16.0, Align::Name);
            x += name_w;
            if let Some(text) = &shape.rollup {
                cell(
                    x,
                    text,
                    size_w + time_w,
                    tokens.text_muted,
                    0.0,
                    Align::Left,
                );
            }
            x += size_w + time_w;
        } else {
            cell(x, &name, name_w, name_color, indent, Align::Name);
            x += name_w;
            cell(x, &size, size_w, tokens.text_muted, 0.0, Align::Right);
            x += size_w;
            cell(x, &when, time_w, tokens.text_muted, 0.0, Align::Left);
            x += time_w;
        }
        if s == Side::Left {
            let marker_rect =
                egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(marker_w, row_h));
            let (icon, meaning, color) = look(item, tokens);
            let response = ui.interact(
                marker_rect,
                Id::NULL.with(("fm-sync-marker", &item.rel)),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::Role::Button,
                    true,
                    format!("{}: {meaning}", item.rel),
                )
            });
            if response.hovered() {
                ui.painter()
                    .rect_filled(marker_rect, tokens.corner_radius, tokens.hover_bg);
            }
            let icon_color = if changing { tokens.bg } else { color };
            if changing {
                let pill = egui::Rect::from_center_size(
                    marker_rect.center(),
                    egui::vec2(38.0, row_h - 6.0),
                );
                ui.painter().rect_filled(pill, row_h, color);
            }
            sync_icons::paint(ui.painter(), icon, marker_rect.center(), 16.0, icon_color);
            let note = item.note.clone();
            let response = response.on_hover_text(match &note {
                Some(n) => format!("{meaning} — {n}"),
                None => meaning.to_string(),
            });
            egui::Popup::menu(&response).show(|ui| {
                for (name, label) in choices(item, shape.policy.0, shape.policy.1) {
                    let label = if is_dir {
                        label
                            .replace("Skip this item", "Skip everything in this folder")
                            .replace("Do nothing", "Do nothing with this folder")
                            .replace("Copy to", "Copy everything to")
                            .replace("Delete on", "Delete everything on")
                    } else {
                        label.to_string()
                    };
                    let clicked =
                        sync_icons::menu_item(ui, tokens, Icon::for_action(name), &label, false)
                            .on_hover_text(action_hint(name))
                            .clicked();
                    if clicked {
                        commands.push((
                            "sync.set_action",
                            json!({ "rel": item.rel, "action": name, "recursive": is_dir }),
                        ));
                        ui.close();
                    }
                }
                if !item.rel.is_empty() && !item.rel.contains(['*', '?']) {
                    ui.separator();
                    let label = if is_dir {
                        "Add folder to ignore list"
                    } else {
                        "Add to ignore list"
                    };
                    if sync_icons::menu_item(ui, tokens, Icon::Funnel, label, false)
                        .on_hover_text(
                            "Leaves this out of every future comparison of this sync (step 5, \
                             Leave out) and compares again.",
                        )
                        .clicked()
                    {
                        commands
                            .push(("sync.ignore", json!({ "rel": item.rel, "folder": is_dir })));
                        ui.close();
                    }
                }
            });
            x += marker_w;
        }
    }
    if is_dir {
        let folder =
            egui::Rect::from_min_size(rect.min, egui::vec2(name_w + size_w + time_w, row_h));
        let fold = ui.interact(
            folder,
            Id::NULL.with(("fm-sync-fold", &item.rel)),
            egui::Sense::click(),
        );
        fold.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                true,
                format!(
                    "{} folder {}",
                    if shape.collapsed {
                        "Expand"
                    } else {
                        "Collapse"
                    },
                    item.rel
                ),
            )
        });
        if fold.clicked() {
            event.toggle = true;
            event.select = true;
        }
    }
    if row_response.clicked() {
        event.select = true;
    }
    row_response.on_hover_text(format!(
        "{}\nLeft: {}\nRight: {}",
        item.rel,
        full_path(shape.left_root, item, item.left.is_some()),
        full_path(shape.right_root, item, item.right.is_some()),
    ));
    event
}

fn full_path(root: &VfsUri, item: &Item, present: bool) -> String {
    if present {
        format!("{}/{}", display_path(root).trim_end_matches('/'), item.rel)
    } else {
        "(does not exist yet)".to_string()
    }
}
