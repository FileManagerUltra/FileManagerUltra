//! The "Compare With…" window: browse any folder (local or remote) and pick the file to compare
//! the selected one with. Browsing and the final comparison are `file.compare_with.*` and
//! `preview.diff.compare` commands.

use egui::{Context, Id, RichText};
use fm_engine::{DirState, Session};
use fm_vfs::{Entry, VfsUri};
use serde_json::json;

use crate::theme::Tokens;

#[derive(Clone, Default)]
struct Draft {
    folder: String,
    path: String,
}

fn draft_id() -> Id {
    Id::NULL.with("fm-compare-with-draft")
}

/// Folders first, then files, each by name; the compared file itself is left out.
fn sorted(entries: &[Entry], source: &VfsUri) -> Vec<Entry> {
    let mut rows: Vec<Entry> = entries
        .iter()
        .filter(|e| e.uri != *source && (e.is_dir_like() || e.is_file()))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        b.is_dir_like()
            .cmp(&a.is_dir_like())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    rows
}

/// Render the window while `s.compare` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(picker) = s.compare.clone() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    let folder_text = picker.folder.to_string();
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .unwrap_or_default();
    if draft.folder != folder_text {
        draft.folder = folder_text.clone();
        draft.path = folder_text.clone();
    }
    let state = s.dirs.state(&picker.folder);
    let rows = s
        .dirs
        .entries(&picker.folder)
        .map(|entries| sorted(&entries, &picker.source))
        .unwrap_or_default();
    let name = picker.source.file_name().unwrap_or_default().to_string();

    let mut open = true;
    let mut go: Option<VfsUri> = None;
    let mut choose: Option<VfsUri> = None;
    let mut run = false;
    let mut cancel = false;
    crate::dialog_style::window(ctx, tokens, "Compare With", &mut open)
        .id(Id::NULL.with("fm-compare-with"))
        .resizable(true)
        .default_size([460.0, 480.0])
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(RichText::new(format!("Compare \"{name}\" with:")).color(tokens.text));
            ui.horizontal(|ui| {
                let parent = picker.folder.parent();
                if ui
                    .add_enabled(parent.is_some(), egui::Button::new("Up"))
                    .on_hover_text("Parent folder")
                    .clicked()
                {
                    go = parent;
                }
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut draft.path)
                        .hint_text("Folder")
                        .desired_width(f32::INFINITY),
                );
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    match VfsUri::parse(draft.path.trim()) {
                        Ok(uri) => go = Some(uri),
                        Err(_) => s.notice = Some(format!("Not a valid location: {}", draft.path)),
                    }
                }
            });
            let footer = tokens.row_height * 2.5;
            egui::ScrollArea::vertical()
                .id_salt("fm-compare-with-list")
                .max_height((ui.available_height() - footer).max(tokens.row_height * 4.0))
                .auto_shrink([false, false])
                .show(ui, |ui| match &state {
                    DirState::Failed { message, .. } => {
                        ui.colored_label(tokens.error, message);
                    }
                    DirState::Ready { .. } => {
                        for entry in &rows {
                            let dir = entry.is_dir_like();
                            let label = if dir {
                                format!("{}/", entry.name)
                            } else {
                                entry.name.clone()
                            };
                            let selected = picker.selected.as_ref() == Some(&entry.uri);
                            let r = ui.add(egui::Button::selectable(selected, label).truncate());
                            if r.double_clicked() {
                                if dir {
                                    go = Some(entry.uri.clone());
                                } else {
                                    choose = Some(entry.uri.clone());
                                    run = true;
                                }
                            } else if r.clicked() && !dir {
                                choose = Some(entry.uri.clone());
                            }
                        }
                        if rows.is_empty() {
                            crate::dialog_style::hint(ui, tokens, "Nothing to compare with here");
                        }
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            crate::dialog_style::hint(ui, tokens, "Reading folder…");
                        });
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                });
            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(picker.selected.is_some(), egui::Button::new("Compare"))
                    .on_hover_text("Show the two files as a diff in the Preview pane")
                    .clicked()
                {
                    run = true;
                }
                if ui.button("Cancel").on_hover_text("Close (Esc)").clicked() {
                    cancel = true;
                }
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel = true;
    }
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    if cancel || !open {
        s.dispatch_or_notice("file.compare_with.close", &serde_json::Value::Null);
        return;
    }
    if let Some(uri) = go {
        s.dispatch_or_notice(
            "file.compare_with.navigate",
            &json!({ "uri": uri.to_string() }),
        );
    }
    if let Some(uri) = choose {
        s.dispatch_or_notice(
            "file.compare_with.select",
            &json!({ "uri": uri.to_string() }),
        );
    }
    if run {
        s.dispatch_or_notice("preview.diff.compare", &serde_json::Value::Null);
    }
}
