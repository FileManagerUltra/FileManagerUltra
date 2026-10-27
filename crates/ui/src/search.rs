//! The Find Files window: a form (folder, name pattern, text inside files, hidden files), the
//! live status of the background walk and its virtualized results. Every action is a command:
//! `search.start`/`stop`/`close`, `file.open`, `file.reveal`, `file.copy_path`.

use egui::{Context, Id, RichText};
use fm_engine::Session;
use fm_engine::item_cmds::display_path;
use fm_vfs::EntryKind;
use serde_json::json;

use crate::listing::human_size;
use crate::theme::Tokens;

/// The form as typed; the engine only sees it when a search starts.
#[derive(Clone, Default)]
struct Draft {
    root: String,
    name: String,
    content: String,
    hidden: bool,
    /// The engine root this draft last copied, so `search.open` on another folder refreshes it.
    synced_root: Option<String>,
    selected: Option<usize>,
    focus: bool,
}

fn draft_id() -> Id {
    Id::NULL.with("fm-search-draft")
}

/// Render the window while `s.search` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(search) = s.search.as_ref() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    let status = search.status();
    let engine_root = search.query.root.as_ref().map(|u| u.to_string());
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .unwrap_or_else(|| Draft {
            name: search.query.name.clone(),
            content: search.query.content.clone(),
            hidden: search.query.hidden,
            focus: true,
            ..Draft::default()
        });
    if draft.synced_root != engine_root {
        draft.root = engine_root.clone().unwrap_or_default();
        draft.synced_root = engine_root;
        draft.focus = true;
    }
    if status.running {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let mut open = true;
    let mut command: Option<(&'static str, serde_json::Value)> = None;
    let mut start = false;
    egui::Window::new("Find Files")
        .id(Id::NULL.with("fm-search"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size([620.0, 420.0])
        .default_pos(ctx.content_rect().center() - egui::Vec2::from([620.0, 420.0]) / 2.0)
        .frame(crate::dialogs::frame(tokens))
        .show(ctx, |ui| {
            let mut submit = false;
            egui::Grid::new("fm-search-form")
                .num_columns(2)
                .show(ui, |ui| {
                    let field = |ui: &mut egui::Ui, label: &str, text: &mut String, hint: &str, tip: &str| {
                        let caption = ui.label(tokens.muted(label));
                        let r = ui
                            .add(
                                egui::TextEdit::singleline(text)
                                    .hint_text(hint)
                                    .desired_width(f32::INFINITY),
                            )
                            .labelled_by(caption.id)
                            .on_hover_text(tip);
                        ui.end_row();
                        r
                    };
                    let r = field(ui, "Look in", &mut draft.root, "folder", "The folder to search below");
                    submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let r = field(
                        ui,
                        "Name",
                        &mut draft.name,
                        "*.txt; report*",
                        "Names containing this text, or matching * ? [a-z]; separate alternatives with ;",
                    );
                    if draft.focus {
                        r.request_focus();
                        draft.focus = false;
                    }
                    submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let r = field(
                        ui,
                        "Containing",
                        &mut draft.content,
                        "text inside files",
                        "Only files containing this text (case-insensitive; binary files are skipped)",
                    );
                    submit |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                });
            ui.horizontal(|ui| {
                ui.checkbox(&mut draft.hidden, "Include hidden")
                    .on_hover_text("Also search hidden files and folders");
                let ready = !draft.name.trim().is_empty() || !draft.content.is_empty();
                if status.running {
                    if ui.button("Stop").on_hover_text("Stop searching").clicked() {
                        command = Some(("search.stop", serde_json::Value::Null));
                    }
                    ui.spinner();
                } else if ui
                    .add_enabled(ready, egui::Button::new("Search"))
                    .on_hover_text("Search (Enter)")
                    .clicked()
                    || (submit && ready)
                {
                    start = true;
                }
            });
            ui.label(tokens.muted(status_line(&status)));
            if let Some(err) = &status.error {
                ui.label(RichText::new(err).color(tokens.error));
            }
            ui.separator();
            let selected_uri = draft
                .selected
                .and_then(|i| search_result(s, i))
                .map(|e| e.uri.to_string());
            ui.horizontal(|ui| {
                let some = selected_uri.is_some();
                if ui
                    .add_enabled(some, egui::Button::new("Open"))
                    .on_hover_text("Open with the default application (double-click)")
                    .clicked()
                    && let Some(uri) = &selected_uri
                {
                    command = Some(("file.open", json!({ "uri": uri })));
                }
                if ui
                    .add_enabled(some, egui::Button::new("Show in Folder"))
                    .on_hover_text("Select the item in a Files pane")
                    .clicked()
                    && let Some(uri) = &selected_uri
                {
                    command = Some(("file.reveal", json!({ "uri": uri })));
                }
                if ui
                    .add_enabled(some, egui::Button::new("Copy Path"))
                    .on_hover_text("Copy the item's path to the clipboard")
                    .clicked()
                    && let Some(uri) = &selected_uri
                {
                    command = Some(("file.copy_path", json!({ "sources": [uri] })));
                }
            });
            let row_height = tokens.row_height;
            egui::ScrollArea::both()
                .id_salt("fm-search-results")
                .auto_shrink([false, false])
                .show_rows(ui, row_height, status.found, |ui, range| {
                    for i in range {
                        let Some(entry) = search_result(s, i) else {
                            continue;
                        };
                        let folder = entry.uri.parent().map(|p| display_path(&p)).unwrap_or_default();
                        let dir = entry.kind == EntryKind::Dir;
                        let size = if dir {
                            String::new()
                        } else {
                            entry.size.map(human_size).unwrap_or_default()
                        };
                        let text = if dir {
                            format!("{}/", entry.name)
                        } else {
                            entry.name.clone()
                        };
                        let response = ui
                            .horizontal(|ui| {
                                let r = ui.add(
                                    egui::Button::selectable(draft.selected == Some(i), text)
                                        .right_text(RichText::new(size).color(tokens.text_muted)),
                                );
                                ui.label(tokens.muted(folder));
                                r
                            })
                            .inner;
                        if response.clicked() {
                            draft.selected = Some(i);
                        }
                        if response.double_clicked() {
                            let uri = entry.uri.to_string();
                            command = Some(if dir {
                                ("file.reveal", json!({ "uri": uri }))
                            } else {
                                ("file.open", json!({ "uri": uri }))
                            });
                        }
                    }
                });
        });
    if start {
        draft.selected = None;
        let mut params = json!({
            "name": draft.name.trim(),
            "content": draft.content,
            "hidden": draft.hidden,
        });
        if !draft.root.trim().is_empty() {
            params["root"] = json!(draft.root.trim());
        }
        s.dispatch_or_notice("search.start", &params);
        draft.synced_root = s
            .search
            .as_ref()
            .and_then(|x| x.query.root.as_ref())
            .map(|u| u.to_string());
    }
    if let Some((id, params)) = command {
        s.dispatch_or_notice(id, &params);
    }
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    if !open {
        s.dispatch_or_notice("search.close", &serde_json::Value::Null);
    }
}

fn search_result(s: &Session, i: usize) -> Option<fm_vfs::Entry> {
    s.search.as_ref().and_then(|x| x.result(i))
}

fn status_line(status: &fm_engine::search_cmds::SearchStatus) -> String {
    let found = match status.found {
        1 => "1 match".to_string(),
        n => format!("{n} matches"),
    };
    let scanned = format!("{} folders, {} files searched", status.dirs, status.files);
    if status.running {
        format!("{found} · {scanned} · {}", status.current)
    } else if status.truncated {
        format!("{found} · stopped at the result limit")
    } else if status.stopped {
        format!("{found} · stopped · {scanned}")
    } else if status.dirs == 0 && status.files == 0 {
        "Enter a name pattern and/or text, then Search.".to_string()
    } else {
        format!("{found} · {scanned}")
    }
}
