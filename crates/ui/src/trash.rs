//! The Trash pane: a table of trashed items inside a pane. Read-only over `Session::trash`;
//! every action is a `trash.*` command, and permanent removal asks for confirmation first.
//! Trashed folders expand inline (`+`/`-`) or open on double-click.

use std::collections::HashSet;

use egui::{Align2, Id, RichText, Sense, Ui};
use fm_engine::Session;
use fm_engine::trash_cmds::{Children, child_key};
use fm_engine::trash_cmds::{TrashChild, TrashChildren, TrashEntry};
use fm_layout::PaneId;
use serde_json::json;

use crate::listing::human_size;
use crate::theme::Tokens;

const INDENT: f32 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pending {
    Purge,
    Empty,
}

fn pending_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-trash-pending", pane))
}

/// `2026-10-25 14:03` (UTC) from Unix seconds.
pub fn date_text(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

fn restore_items(s: &Session) -> serde_json::Value {
    let mut items: Vec<_> = s.trash.selected_children.iter().collect();
    items.sort();
    json!({"items": items.iter().map(|(id, path)| json!({"id": id, "path": path})).collect::<Vec<_>>()})
}

fn sorted(set: &HashSet<String>) -> Vec<&String> {
    let mut v: Vec<_> = set.iter().collect();
    v.sort();
    v
}

enum Kind<'a> {
    Top(&'a TrashEntry),
    Child {
        id: &'a str,
        path: Vec<String>,
        child: &'a TrashChild,
    },
    Note(String, bool),
}

struct Row<'a> {
    depth: usize,
    kind: Kind<'a>,
}

fn collect_children<'a>(
    s: &'a Session,
    id: &'a str,
    path: &mut Vec<String>,
    depth: usize,
    rows: &mut Vec<Row<'a>>,
) {
    match s.trash.expanded.get(&child_key(id, path)) {
        None => {}
        Some(Children::Loading) => rows.push(note(depth, "Loading…".to_string(), false)),
        Some(Children::Failed(e)) => rows.push(note(depth, e.clone(), true)),
        Some(Children::Ready(c)) => push_children(s, id, path, depth, c, rows),
    }
}

fn note(depth: usize, text: String, error: bool) -> Row<'static> {
    Row {
        depth,
        kind: Kind::Note(text, error),
    }
}

fn push_children<'a>(
    s: &'a Session,
    id: &'a str,
    path: &mut Vec<String>,
    depth: usize,
    children: &'a TrashChildren,
    rows: &mut Vec<Row<'a>>,
) {
    for child in &children.entries {
        path.push(child.name.clone());
        rows.push(Row {
            depth,
            kind: Kind::Child {
                id,
                path: path.clone(),
                child,
            },
        });
        if child.is_dir {
            collect_children(s, id, path, depth + 1, rows);
        }
        path.pop();
    }
    if children.truncated {
        rows.push(note(depth, "More items not shown".to_string(), false));
    }
}

fn build_rows(s: &Session) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    if let Some(v) = &s.trash.view {
        let mut path = v.path.clone();
        match &v.children {
            Children::Loading => rows.push(note(0, "Loading…".to_string(), false)),
            Children::Failed(e) => rows.push(note(0, e.clone(), true)),
            Children::Ready(c) => push_children(s, &v.id, &mut path, 0, c, &mut rows),
        }
        return rows;
    }
    for e in &s.trash.listing.entries {
        rows.push(Row {
            depth: 0,
            kind: Kind::Top(e),
        });
        if e.is_dir {
            collect_children(s, &e.id, &mut Vec::new(), 1, &mut rows);
        }
    }
    rows
}

/// Render the pane body. Call only for `PaneKind::Trash`.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    if !s.trash.loaded && !s.trash.busy {
        s.dispatch_or_notice("trash.refresh", &json!({}));
    }
    let pid = pending_id(pane);
    let mut pending = ui
        .ctx()
        .data(|d| d.get_temp::<Option<Pending>>(pid))
        .flatten();
    let mut cmds: Vec<(&'static str, serde_json::Value)> = Vec::new();
    let mut clicks: Vec<(String, Vec<String>, bool)> = Vec::new();

    let count = s.trash.listing.entries.len();
    let selected = s.trash.selected.len();
    let picked = s.trash.selected_children.len();
    let idle = !s.trash.busy;
    let in_view = s.trash.view.is_some();
    ui.horizontal(|ui| {
        if in_view {
            if ui.button("Back").clicked() {
                cmds.push(("trash.leave", json!({})));
            }
            if ui
                .add_enabled(idle && picked > 0, egui::Button::new("Restore"))
                .clicked()
            {
                cmds.push(("trash.restore_children", restore_items(s)));
            }
            if let Some(v) = &s.trash.view {
                let mut here = v.name.clone();
                if v.path.len() > 1 {
                    here = v.path.join("/");
                }
                ui.label(tokens.muted(format!("Trash / {here}")));
            }
            return;
        }
        if ui.add_enabled(idle, egui::Button::new("Refresh")).clicked() {
            cmds.push(("trash.refresh", json!({})));
        }
        if ui
            .add_enabled(idle && selected + picked > 0, egui::Button::new("Restore"))
            .clicked()
        {
            if picked > 0 {
                cmds.push(("trash.restore_children", restore_items(s)));
            } else {
                cmds.push(("trash.restore", json!({"ids": sorted(&s.trash.selected)})));
            }
        }
        if ui
            .add_enabled(
                idle && selected > 0,
                egui::Button::new("Delete Permanently"),
            )
            .clicked()
        {
            pending = Some(Pending::Purge);
        }
        if ui
            .add_enabled(idle && count > 0, egui::Button::new("Empty Trash"))
            .clicked()
        {
            pending = Some(Pending::Empty);
        }
    });
    if let Some(kind) = pending.filter(|_| !in_view) {
        ui.horizontal(|ui| {
            let text = match kind {
                Pending::Purge => format!("Permanently delete {selected} selected?"),
                Pending::Empty => format!("Permanently delete all {count} items?"),
            };
            ui.label(RichText::new(text).color(tokens.error).font(tokens.font()));
            if ui.button("Confirm").clicked() {
                match kind {
                    Pending::Purge => cmds.push((
                        "trash.purge",
                        json!({"ids": sorted(&s.trash.selected), "confirm": true}),
                    )),
                    Pending::Empty => cmds.push(("trash.empty", json!({"confirm": true}))),
                }
                pending = None;
            }
            if ui.button("Cancel").clicked() {
                pending = None;
            }
        });
    }
    if let Some(e) = &s.trash.error {
        ui.label(RichText::new(e).color(tokens.error).font(tokens.font()));
    }
    if let Some(n) = &s.trash.notice {
        ui.label(tokens.muted(n));
    }
    let status = if s.trash.busy {
        "Working…".to_string()
    } else if in_view {
        "Items inside a trashed folder (read-only; restore the folder to get them back)".to_string()
    } else if s.trash.listing.truncated {
        format!("{count} items shown (the trash holds more)")
    } else {
        format!("{count} items, {selected} selected")
    };
    ui.label(tokens.muted(&status));
    ui.separator();

    let width = ui.available_width();
    let name_w = (width * 0.35).max(120.0);
    let dir_w = (width * 0.40).max(120.0);
    let row_h = tokens.row_height;
    let (head, _) = ui.allocate_exact_size(egui::vec2(width, row_h), Sense::hover());
    let titles = if in_view {
        [(0.0, "Name"), (name_w, "Size"), (name_w + dir_w, "")]
    } else {
        [
            (0.0, "Name"),
            (name_w, "Original folder"),
            (name_w + dir_w, "Deleted"),
        ]
    };
    for (x, title) in titles {
        ui.painter().text(
            head.left_center() + egui::vec2(x + 6.0, 0.0),
            Align2::LEFT_CENTER,
            title,
            tokens.font(),
            tokens.text_muted,
        );
    }
    let rows = build_rows(s);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_h, rows.len(), |ui, range| {
            let ctrl = ui.input(|i| i.modifiers.command);
            for row in rows.get(range).unwrap_or_default() {
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), row_h), Sense::click());
                let indent = row.depth as f32 * INDENT;
                let on = match &row.kind {
                    Kind::Top(e) => s.trash.selected.contains(&e.id),
                    Kind::Child { id, path, .. } => s
                        .trash
                        .selected_children
                        .contains(&(id.to_string(), path.clone())),
                    Kind::Note(..) => false,
                };
                let fg = if on {
                    tokens.selection_text
                } else {
                    tokens.text
                };
                if on {
                    ui.painter().rect_filled(rect, 0.0, tokens.selection_bg);
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, tokens.hover_bg);
                }
                let clip = ui.painter().with_clip_rect(rect);
                let cell = |x: f32, w: f32, text: &str, color: egui::Color32| {
                    clip.with_clip_rect(egui::Rect::from_min_size(
                        rect.min + egui::vec2(x, 0.0),
                        egui::vec2(w - 4.0, row_h),
                    ))
                    .text(
                        rect.left_center() + egui::vec2(x + 6.0, 0.0),
                        Align2::LEFT_CENTER,
                        text,
                        tokens.font(),
                        color,
                    );
                };
                // (id, path, name, is_dir, name text)
                let target: Option<(&str, Vec<String>, &str, bool)> = match &row.kind {
                    Kind::Top(e) => Some((e.id.as_str(), Vec::new(), e.name.as_str(), e.is_dir)),
                    Kind::Child { id, path, child } => {
                        Some((id, path.clone(), child.name.as_str(), child.is_dir))
                    }
                    Kind::Note(..) => None,
                };
                match &row.kind {
                    Kind::Top(e) => {
                        cell(name_w, dir_w, &e.original_parent, fg);
                        cell(name_w + dir_w, width, &date_text(e.deleted), fg);
                    }
                    Kind::Child { child, .. } => {
                        if !child.is_dir {
                            cell(name_w, dir_w, &human_size(child.size), tokens.text_muted);
                        }
                    }
                    Kind::Note(text, error) => {
                        let color = if *error {
                            tokens.error
                        } else {
                            tokens.text_muted
                        };
                        cell(indent, width, text, color);
                    }
                }
                let Some((id, path, name, is_dir)) = target else {
                    continue;
                };
                let open = is_dir && s.trash.expanded.contains_key(&child_key(id, &path));
                let marker = match (is_dir, open) {
                    (false, _) => "",
                    (true, false) => "+",
                    (true, true) => "-",
                };
                let label = format!("{marker:<2}{name}");
                clip.with_clip_rect(egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(name_w - 4.0, row_h),
                ))
                .text(
                    rect.left_center() + egui::vec2(indent + 6.0, 0.0),
                    Align2::LEFT_CENTER,
                    label,
                    tokens.font(),
                    fg,
                );
                let arrow = egui::Rect::from_min_size(
                    rect.min + egui::vec2(indent, 0.0),
                    egui::vec2(INDENT + 4.0, row_h),
                );
                let on_arrow = resp
                    .interact_pointer_pos()
                    .is_some_and(|p| arrow.contains(p));
                if resp.double_clicked() && is_dir && !on_arrow {
                    cmds.push(("trash.enter", json!({"id": id, "path": path, "name": name})));
                } else if resp.clicked() {
                    if is_dir && on_arrow {
                        let cmd = if open {
                            "trash.collapse"
                        } else {
                            "trash.expand"
                        };
                        cmds.push((cmd, json!({"id": id, "path": path})));
                    } else {
                        clicks.push((id.to_string(), path.clone(), ctrl));
                    }
                }
            }
        });

    for (id, path, additive) in clicks {
        let child = !path.is_empty();
        let other_kind = if child {
            !s.trash.selected.is_empty()
        } else {
            !s.trash.selected_children.is_empty()
        };
        if !additive || other_kind {
            s.trash.selected.clear();
            s.trash.selected_children.clear();
        }
        if child {
            let key = (id, path);
            if !s.trash.selected_children.remove(&key) {
                s.trash.selected_children.insert(key);
            }
        } else if !s.trash.selected.remove(&id) {
            s.trash.selected.insert(id);
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(pid, pending));
    for (id, params) in cmds {
        s.dispatch_or_notice(id, &params);
    }
}

#[cfg(test)]
mod tests {
    use super::date_text;

    #[test]
    fn formats_unix_seconds() {
        assert_eq!(date_text(0), "1970-01-01 00:00");
        assert_eq!(date_text(1_782_393_780), "2026-06-25 13:23");
    }
}
