//! Modal dialogs of the file operations: conflict resolution, permanent-delete confirmation,
//! new-folder / rename name entry and the copy/move destination prompt. Dialogs only
//! collect input; everything is dispatched as commands (`job.resolve_conflict`,
//! `file.mkdir`, ...). Colours come from tokens.

use egui::{Context, Id, Key, RichText};
use fm_engine::{Session, file_cmds};
use fm_vfs::{Scheme, VfsUri};
use serde_json::json;

use crate::theme::Tokens;

#[derive(Clone, Debug, PartialEq)]
pub enum Dialog {
    Mkdir {
        dir: VfsUri,
        name: String,
    },
    NewFile {
        dir: VfsUri,
        name: String,
    },
    Rename {
        dir: VfsUri,
        current: String,
        name: String,
    },
    ConfirmDelete {
        sources: Vec<VfsUri>,
    },
    /// Rename a workspace tab (0-based position).
    RenameWorkspace {
        tab: usize,
        name: String,
    },
    /// Double-clicked program: run it, open it as a document, or do nothing.
    ConfirmRun {
        uri: VfsUri,
        name: String,
    },
    /// Rename a bookmark in Places.
    RenameBookmark {
        uri: String,
        name: String,
    },
    /// Select (or deselect) the names matching a wildcard pattern.
    SelectPattern {
        pattern: String,
        mode: &'static str,
    },
    /// Copy/move with no other pane to take as destination.
    Destination {
        command: &'static str,
        sources: Vec<VfsUri>,
        dest: String,
    },
}

/// Open-dialog state held by the app.
#[derive(Default)]
pub struct Dialogs {
    pub open: Option<Dialog>,
    focus_next: bool,
}

fn queue_id() -> Id {
    Id::NULL.with("fm-dialog-queue")
}

/// Ask for `dialog` from a widget that has no access to [`Dialogs`]; it opens on the next
/// [`Dialogs::show`].
pub fn queue(ctx: &Context, dialog: Dialog) {
    ctx.data_mut(|d| d.insert_temp(queue_id(), Some(dialog)));
}

/// The dialog a command needs before it can run, if any. The app calls this for shortcuts and
/// menu items; `None` means dispatch the command directly.
pub fn dialog_for(id: &str, s: &Session) -> Option<Dialog> {
    if id == "workspace.rename" {
        let tab = s.workspaces.active;
        return Some(Dialog::RenameWorkspace {
            tab,
            name: s.workspaces.tabs.get(tab)?.name.clone(),
        });
    }
    let dir = file_cmds::focused_dir(s)?;
    let selection = file_cmds::focused_selection(s);
    match id {
        "edit.select_pattern" => Some(Dialog::SelectPattern {
            pattern: "*".to_string(),
            mode: "replace",
        }),
        "file.mkdir" => Some(Dialog::Mkdir {
            dir: dir.clone(),
            name: String::new(),
        }),
        "file.newfile" => Some(Dialog::NewFile {
            dir: dir.clone(),
            name: String::new(),
        }),
        "file.rename" if selection.len() == 1 => {
            let current = selection.first()?.file_name()?.to_string();
            Some(Dialog::Rename {
                dir: dir.clone(),
                name: current.clone(),
                current,
            })
        }
        "file.delete_permanent" if !selection.is_empty() => {
            Some(Dialog::ConfirmDelete { sources: selection })
        }
        // Remote locations have no trash: Delete asks for a permanent delete instead.
        "file.delete" if selection.iter().any(|u| u.scheme() != Scheme::File) => {
            Some(Dialog::ConfirmDelete { sources: selection })
        }
        "file.copy" | "file.move" if !selection.is_empty() => {
            if file_cmds::other_pane_dir(s).is_some() {
                return None;
            }
            Some(Dialog::Destination {
                command: if id == "file.copy" {
                    "file.copy"
                } else {
                    "file.move"
                },
                sources: selection,
                dest: dir.to_string(),
            })
        }
        _ => None,
    }
}

impl Dialogs {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    pub fn request(&mut self, dialog: Dialog) {
        self.open = Some(dialog);
        self.focus_next = true;
    }

    /// Render the open dialog and the conflict prompt. Call once per frame.
    pub fn show(&mut self, ctx: &Context, s: &mut Session, tokens: &Tokens) {
        conflict_dialog(ctx, s, tokens);
        kind_switch_dialog(ctx, s, tokens);
        unsaved_dialog(ctx, s, tokens);
        if let Some(Some(dialog)) = ctx.data_mut(|d| d.remove_temp::<Option<Dialog>>(queue_id())) {
            self.request(dialog);
        }
        let Some(mut dialog) = self.open.take() else {
            return;
        };
        let esc = ctx.input(|i| i.key_pressed(Key::Escape));
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let focus = std::mem::take(&mut self.focus_next);
        let keep = match &mut dialog {
            Dialog::Mkdir { dir, name } => {
                let verdict = file_cmds::validate_new_name(s, dir, name, None);
                let (go, cancel) = name_dialog(ctx, tokens, "New folder", name, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    s.dispatch_or_notice(
                        "file.mkdir",
                        &json!({"name": name.as_str(), "parent": dir.to_string()}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::NewFile { dir, name } => {
                let verdict = file_cmds::validate_new_name(s, dir, name, None);
                let (go, cancel) = name_dialog(ctx, tokens, "New file", name, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    s.dispatch_or_notice(
                        "file.newfile",
                        &json!({"name": name.as_str(), "parent": dir.to_string()}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::Rename { dir, current, name } => {
                let verdict = file_cmds::validate_new_name(s, dir, name, Some(current));
                let (go, cancel) = name_dialog(ctx, tokens, "Rename", name, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    if name != current
                        && let Ok(uri) = dir.join(current)
                    {
                        s.dispatch_or_notice(
                            "file.rename",
                            &json!({"name": name.as_str(), "uri": uri.to_string()}),
                        );
                    }
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::RenameWorkspace { tab, name } => {
                let verdict = fm_engine::workspace::clean_name(name)
                    .map(|_| ())
                    .map_err(|_| "1-40 characters, no control characters".to_string());
                let (go, cancel) =
                    name_dialog(ctx, tokens, "Rename workspace", name, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    s.dispatch_or_notice(
                        "workspace.rename",
                        &json!({"tab": *tab, "name": name.as_str()}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::RenameBookmark { uri, name } => {
                let verdict = if name.trim().is_empty()
                    || name.chars().count() > fm_engine::prefs::MAX_BOOKMARK_NAME
                {
                    Err("1-100 characters".to_string())
                } else {
                    Ok(())
                };
                let (go, cancel) =
                    name_dialog(ctx, tokens, "Rename bookmark", name, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    s.dispatch_or_notice(
                        "bookmark.rename",
                        &json!({"uri": uri.as_str(), "name": name.trim()}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::SelectPattern { pattern, mode } => {
                let verdict = fm_engine::glob::Glob::parse(pattern)
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                let (go, cancel) = pattern_dialog(ctx, tokens, pattern, mode, &verdict, focus);
                if !cancel && !esc && (go || enter) && verdict.is_ok() {
                    s.dispatch_or_notice(
                        "edit.select_pattern",
                        &json!({"pattern": pattern.as_str(), "mode": *mode}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::ConfirmDelete { sources } => {
                let (go, cancel) = if s.prefs.confirm_delete {
                    confirm_delete(ctx, tokens, sources.len())
                } else {
                    (true, false)
                };
                if go {
                    let list: Vec<String> = sources.iter().map(|u| u.to_string()).collect();
                    s.dispatch_or_notice(
                        "file.delete_permanent",
                        &json!({"confirm": true, "sources": list}),
                    );
                    false
                } else {
                    !(cancel || esc)
                }
            }
            Dialog::ConfirmRun { uri, name } => {
                let (choice, cancel) = if s.prefs.confirm_run {
                    confirm_run(ctx, tokens, name)
                } else {
                    (Some(true), false)
                };
                let uri_s = uri.to_string();
                match choice {
                    Some(run) => {
                        s.dispatch_or_notice(
                            "file.open",
                            &json!({
                                "uri": uri_s,
                                "mode": if run { "run" } else { "default" },
                                "confirm": true,
                            }),
                        );
                        false
                    }
                    None => !(cancel || esc),
                }
            }
            Dialog::Destination {
                command,
                sources,
                dest,
            } => {
                let valid = VfsUri::parse(dest).is_ok();
                let verdict = if valid {
                    Ok(())
                } else {
                    Err("Not a valid location".to_string())
                };
                let title = if *command == "file.copy" {
                    "Copy to"
                } else {
                    "Move to"
                };
                let (go, cancel) = name_dialog(ctx, tokens, title, dest, &verdict, focus);
                if !cancel && !esc && (go || enter) && valid {
                    let list: Vec<String> = sources.iter().map(|u| u.to_string()).collect();
                    s.dispatch_or_notice(command, &json!({"sources": list, "dest": dest.as_str()}));
                    false
                } else {
                    !(cancel || esc)
                }
            }
        };
        if keep {
            self.open = Some(dialog);
        }
    }
}

pub(crate) fn frame(tokens: &Tokens) -> egui::Frame {
    egui::Frame::window(&egui::Style::default())
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(tokens.border_width, tokens.border))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::same(tokens.padding as i8 * 2))
}

/// A text-entry dialog; returns `(confirmed, cancelled)`.
fn name_dialog(
    ctx: &Context,
    tokens: &Tokens,
    title: &str,
    text: &mut String,
    verdict: &Result<(), String>,
    focus: bool,
) -> (bool, bool) {
    let (mut go, mut cancel) = (false, false);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, title, &mut open)
        .id(Id::NULL.with("fm-name-dialog"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            let caption = ui.label(RichText::new(title).color(tokens.text));
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(280.0));
            let edit = edit.labelled_by(caption.id);
            if focus {
                edit.request_focus();
            }
            match verdict {
                Ok(()) => {
                    ui.label(" ");
                }
                Err(msg) => {
                    ui.label(RichText::new(msg).color(tokens.error));
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(verdict.is_ok(), egui::Button::new("OK"))
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    (go && open, cancel || !open)
}

/// Select by pattern: the pattern plus whether it replaces, extends or trims the selection.
fn pattern_dialog(
    ctx: &Context,
    tokens: &Tokens,
    pattern: &mut String,
    mode: &mut &'static str,
    verdict: &Result<(), String>,
    focus: bool,
) -> (bool, bool) {
    let (mut go, mut cancel) = (false, false);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Select by pattern", &mut open)
        .id(Id::NULL.with("fm-pattern-dialog"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            let caption = ui.label(
                RichText::new("Names matching (* ? [a-z], alternatives split by ;)")
                    .color(tokens.text),
            );
            let edit = ui.add(egui::TextEdit::singleline(pattern).desired_width(280.0));
            let edit = edit.labelled_by(caption.id);
            if focus {
                edit.request_focus();
            }
            ui.horizontal(|ui| {
                for (value, label, tip) in [
                    ("replace", "Select", "Select only the matching items"),
                    ("add", "Add", "Add the matching items to the selection"),
                    (
                        "remove",
                        "Deselect",
                        "Remove the matching items from the selection",
                    ),
                ] {
                    ui.radio_value(mode, value, label).on_hover_text(tip);
                }
            });
            match verdict {
                Ok(()) => {
                    ui.label(" ");
                }
                Err(msg) => {
                    ui.label(RichText::new(msg).color(tokens.error));
                }
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(verdict.is_ok(), egui::Button::new("OK"))
                    .clicked()
                {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    (go && open, cancel || !open)
}

/// Quitting or closing a workspace tab that has changes the user has not saved.
fn unsaved_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(prompt) = s.unsaved_prompt.clone() else {
        return;
    };
    let quitting = prompt.after == fm_engine::workspace::AfterPrompt::Quit;
    let mut choice: Option<&str> = None;
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Unsaved workspace changes", &mut open)
        .id(Id::NULL.with("fm-unsaved-workspaces"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            let noun = if prompt.names.len() == 1 {
                "workspace has"
            } else {
                "workspaces have"
            };
            ui.label(RichText::new(format!("These {noun} unsaved changes:")).color(tokens.text));
            for name in &prompt.names {
                ui.label(RichText::new(format!("  {name}")).color(tokens.warning));
            }
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    choice = Some("save");
                }
                if ui
                    .button(if quitting {
                        "Quit without saving"
                    } else {
                        "Close without saving"
                    })
                    .clicked()
                {
                    choice = Some("discard");
                }
                if ui.button("Cancel").clicked() {
                    choice = Some("cancel");
                }
            });
        });
    if !open || ctx.input(|i| i.key_pressed(Key::Escape)) {
        choice = Some("cancel");
    }
    if let Some(choice) = choice {
        s.dispatch_or_notice("workspace.prompt", &json!({ "choice": choice }));
        if s.unsaved_prompt.is_some() && choice == "save" {
            // Saving failed (the notice says why); do not leave the dialog stuck open.
            s.unsaved_prompt = None;
        }
    }
}

fn confirm_delete(ctx: &Context, tokens: &Tokens, count: usize) -> (bool, bool) {
    let (mut go, mut cancel) = (false, false);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Delete permanently", &mut open)
        .id(Id::NULL.with("fm-confirm-delete"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            let noun = if count == 1 { "item" } else { "items" };
            ui.label(
                RichText::new(format!("Permanently delete {count} {noun}?")).color(tokens.text),
            );
            ui.label(
                RichText::new("This cannot be undone and bypasses the trash.").color(tokens.error),
            );
            ui.horizontal(|ui| {
                if ui.button("Delete").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    (go && open, cancel || !open)
}

/// `Some(true)` = run, `Some(false)` = open with the default application.
fn confirm_run(ctx: &Context, tokens: &Tokens, name: &str) -> (Option<bool>, bool) {
    let (mut choice, mut cancel) = (None, false);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "Run program?", &mut open)
        .id(Id::NULL.with("fm-confirm-run"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(RichText::new(format!("\"{name}\" is a program.")).color(tokens.text));
            ui.label(RichText::new("Only run programs you trust.").color(tokens.warning));
            ui.horizontal(|ui| {
                if ui.button("Run").clicked() {
                    choice = Some(true);
                }
                if ui.button("Open as document").clicked() {
                    choice = Some(false);
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    (if open { choice } else { None }, cancel || !open)
}

/// Asks before a pane type switch ends a terminal that is running a command.
fn kind_switch_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some((pane, kind)) = s.pending_kind else {
        return;
    };
    let esc = ctx.input(|i| i.key_pressed(Key::Escape));
    let (mut go, mut cancel) = (false, esc);
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "End running process?", &mut open)
        .id(Id::NULL.with("fm-confirm-kind-switch"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.label(RichText::new("This terminal is running a command.").color(tokens.text));
            ui.label(
                RichText::new(format!(
                    "Switching to {} ends the terminal and everything running in it.",
                    kind.label()
                ))
                .color(tokens.error),
            );
            ui.horizontal(|ui| {
                if ui.button("End and switch").clicked() {
                    go = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if !open || cancel {
        s.pending_kind = None;
    } else if go {
        s.dispatch_or_notice(
            "pane.kind.set",
            &json!({"kind": kind.as_str(), "pane": pane, "confirm": true}),
        );
        s.pending_kind = None;
    }
}

fn conflict_dialog(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(job) = s.fileops.pending_conflict() else {
        return;
    };
    let Some(conflict) = job.conflict.clone() else {
        return;
    };
    let id = job.id;
    let mut choice: Option<&'static str> = None;
    let mut all = ctx.data_mut(|d| {
        d.get_temp::<bool>(Id::NULL.with("fm-conflict-all"))
            .unwrap_or(false)
    });
    let mut open = true;
    crate::dialog_style::window(ctx, tokens, "File exists", &mut open)
        .id(Id::NULL.with("fm-conflict-dialog"))
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            let name = conflict.existing.uri.file_name().unwrap_or("item");
            ui.label(
                RichText::new(format!("\"{name}\" already exists in the destination."))
                    .color(tokens.text),
            );
            for (label, side) in [
                ("Existing", &conflict.existing),
                ("Incoming", &conflict.incoming),
            ] {
                let size = side
                    .size
                    .map(crate::listing::human_size)
                    .unwrap_or_else(|| "unknown size".to_string());
                let when = side
                    .modified
                    .map(crate::listing::format_mtime)
                    .unwrap_or_default();
                crate::dialog_style::hint(ui, tokens, format!("{label}: {size}  {when}"));
            }
            ui.checkbox(&mut all, "Apply to all");
            ui.horizontal(|ui| {
                for (text, value) in [
                    ("Overwrite", "overwrite"),
                    ("Keep both", "keep_both"),
                    ("Skip", "skip"),
                    ("Cancel", "cancel"),
                ] {
                    if ui.button(text).clicked() {
                        choice = Some(value);
                    }
                }
            });
        });
    if !open || ctx.input(|i| i.key_pressed(Key::Escape)) {
        choice = Some("cancel");
    }
    ctx.data_mut(|d| d.insert_temp(Id::NULL.with("fm-conflict-all"), all));
    if let Some(choice) = choice {
        s.dispatch_or_notice(
            "job.resolve_conflict",
            &json!({"job": id, "choice": choice, "apply_to_all": all}),
        );
    }
}
