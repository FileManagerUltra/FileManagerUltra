//! The path picker window: browse any folder (local or remote) and choose a folder or a file.
//! Browsing and choosing are `picker.*` commands.

use egui::{Context, Id, RichText};
use fm_engine::picker_cmds::PickKind;
use fm_engine::{DirState, Session};
use fm_vfs::{Entry, VfsUri};
use serde_json::json;

use crate::sync_icons::{self, Icon};
use crate::theme::Tokens;

#[derive(Clone, Default)]
struct Draft {
    folder: String,
    path: String,
    /// The name being typed for a new folder; `None` while the row is closed.
    creating: Option<String>,
    /// Which chooser (purpose, depth in the stack) the window last showed.
    shown_for: Option<(String, usize)>,
}

fn draft_id() -> Id {
    Id::NULL.with("fm-picker-draft")
}

/// Folders first, then (for file pickers only) files, each by name; hidden entries are left out.
fn sorted(entries: &[Entry], kind: PickKind) -> Vec<Entry> {
    let mut rows: Vec<Entry> = entries
        .iter()
        .filter(|e| !fm_engine::is_hidden(e))
        .filter(|e| e.is_dir_like() || (kind == PickKind::File && e.is_file()))
        .cloned()
        .collect();
    rows.sort_by(|a, b| {
        b.is_dir_like()
            .cmp(&a.is_dir_like())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    rows
}

fn shown(uri: &VfsUri) -> String {
    fm_engine::item_cmds::display_path(uri)
}

/// Render the window while `s.picker` is open. Call once per frame.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    let Some(picker) = s.picker.clone() else {
        ctx.data_mut(|d| d.remove_temp::<Draft>(draft_id()));
        return;
    };
    let folder_text = shown(&picker.folder);
    let mut draft = ctx
        .data_mut(|d| d.get_temp::<Draft>(draft_id()))
        .unwrap_or_default();
    if draft.folder != folder_text {
        draft.folder = folder_text.clone();
        draft.path = folder_text;
        draft.creating = None;
    }
    fm_engine::picker_cmds::poll(s);
    let sections = fm_engine::picker_cmds::places(s);
    let state = s.dirs.state(&picker.folder);
    let rows = s
        .dirs
        .entries(&picker.folder)
        .map(|entries| sorted(&entries, picker.kind))
        .unwrap_or_default();
    let folder_kind = picker.kind == PickKind::Folder;
    let title = if !picker.title.is_empty() {
        picker.title.clone()
    } else if folder_kind {
        "Choose a folder".to_string()
    } else {
        "Choose a file".to_string()
    };

    let mut open = true;
    let mut go: Option<VfsUri> = None;
    let mut choose: Option<VfsUri> = None;
    let mut accept = false;
    let mut cancel = false;
    let mut create: Option<String> = None;
    let mut escape_used = false;
    // A chooser opened from another dialog's Browse… must stay above that dialog, which lives
    // on the Foreground layer; the first chooser stays below the dialogs it opens.
    let layer = if picker.nested {
        egui::Order::Foreground
    } else {
        egui::Order::Middle
    };
    crate::dialog_style::window(ctx, tokens, &title, &mut open)
        .order(layer)
        .id(Id::NULL.with("fm-picker"))
        .resizable(false)
        .fixed_size([780.0, 480.0])
        .show(ctx, |ui| {
            crate::dialog_style::apply(ui, tokens);
            ui.horizontal_top(|ui| {
                ui.allocate_ui(egui::vec2(220.0, 400.0), |ui| {
                    ui.set_width(220.0);
                    ui.vertical(|ui| {
                        if let Some(uri) = places_column(ui, tokens, &sections, &picker.folder) {
                            go = Some(uri);
                        }
                    });
                });
                ui.allocate_ui(egui::vec2(510.0, 400.0), |ui| {
                    ui.set_width(510.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            let parent = picker.folder.parent();
                            if ui
                                .add_enabled_ui(parent.is_some(), |ui| {
                                    sync_icons::button(ui, tokens, Icon::Back, "Up", false)
                                        .on_hover_text("Go to the parent folder")
                                        .clicked()
                                })
                                .inner
                            {
                                go = parent;
                            }
                            let can_create = fm_engine::picker_cmds::choosable(&picker.folder);
                            if ui
                                .add_enabled(can_create, egui::Button::new("New folder"))
                                .on_hover_text("Create a folder in the open folder")
                                .clicked()
                            {
                                draft.creating = Some(String::new());
                            }
                            let edit = ui.add(
                                egui::TextEdit::singleline(&mut draft.path)
                                    .hint_text("Type a location and press Enter")
                                    .margin(egui::vec2(8.0, 6.0))
                                    .desired_width(f32::INFINITY),
                            );
                            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                match fm_sync::profile::parse_endpoint(draft.path.trim()) {
                                    Ok(uri) => go = Some(uri),
                                    Err(_) => {
                                        s.notice =
                                            Some(format!("Not a valid location: {}", draft.path))
                                    }
                                }
                            }
                        });
                        if let Some(name) = draft.creating.as_mut() {
                            ui.horizontal(|ui| {
                                let edit = ui.add(
                                    egui::TextEdit::singleline(name)
                                        .hint_text("New folder name")
                                        .margin(egui::vec2(8.0, 6.0))
                                        .desired_width(ui.available_width() - 190.0),
                                );
                                if !edit.has_focus() && !edit.lost_focus() {
                                    edit.request_focus();
                                }
                                let valid = fm_engine::file_cmds::validate_new_name(
                                    s,
                                    &picker.folder,
                                    name.trim(),
                                    None,
                                );
                                let enter = edit.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                let clicked = ui
                                    .add_enabled(valid.is_ok(), egui::Button::new("Create"))
                                    .clicked();
                                if (clicked || enter) && valid.is_ok() {
                                    create = Some(name.trim().to_string());
                                }
                                if let Err(why) = &valid
                                    && !name.trim().is_empty()
                                {
                                    ui.colored_label(tokens.error, why);
                                }
                                if ui.button("Cancel").clicked() {
                                    escape_used = true;
                                }
                            });
                        }
                        egui::ScrollArea::vertical()
                            .id_salt("fm-picker-list")
                            .max_height(270.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.spacing_mut().interact_size.y = 28.0;
                                match &state {
                                    DirState::Failed { message, .. } => {
                                        ui.colored_label(tokens.error, message);
                                        crate::dialog_style::hint(
                                            ui,
                                            tokens,
                                            "Go up or type another location.",
                                        );
                                    }
                                    DirState::Ready { .. } => {
                                        for entry in &rows {
                                            let dir = entry.is_dir_like();
                                            let selected =
                                                picker.selected.as_ref() == Some(&entry.uri);
                                            let r = ui
                                                .horizontal(|ui| {
                                                    if dir {
                                                        sync_icons::icon(
                                                            ui,
                                                            tokens,
                                                            Icon::Folder,
                                                            18.0,
                                                        );
                                                    } else {
                                                        ui.add_space(22.0);
                                                    }
                                                    let row = ui.add_sized(
                                                        [ui.available_width(), 28.0],
                                                        egui::Button::selectable(selected, "")
                                                            .left_text(entry.name.as_str())
                                                            .truncate(),
                                                    );
                                                    row.widget_info(|| {
                                                        egui::WidgetInfo::labeled(
                                                            egui::Role::Button,
                                                            true,
                                                            entry.name.as_str(),
                                                        )
                                                    });
                                                    row
                                                })
                                                .inner;
                                            if r.double_clicked() {
                                                if dir {
                                                    go = Some(entry.uri.clone());
                                                } else {
                                                    choose = Some(entry.uri.clone());
                                                    accept = true;
                                                }
                                            } else if r.clicked() {
                                                choose = Some(entry.uri.clone());
                                            }
                                        }
                                        if rows.is_empty() {
                                            crate::dialog_style::hint(
                                                ui,
                                                tokens,
                                                "Nothing to choose here",
                                            );
                                        }
                                    }
                                    _ => {
                                        ui.horizontal(|ui| {
                                            ui.add(crate::spinner::Spinner::new());
                                            crate::dialog_style::hint(
                                                ui,
                                                tokens,
                                                "Reading folder…",
                                            );
                                        });
                                        ui.ctx().request_repaint_after(
                                            std::time::Duration::from_millis(100),
                                        );
                                    }
                                }
                            });
                        ui.separator();
                        let hint = match (&picker.selected, folder_kind) {
                            (Some(sel), _) => format!("Will choose: {}", shown(sel)),
                            (None, true) => {
                                format!("Will choose the open folder: {}", shown(&picker.folder))
                            }
                            (None, false) => "Select a file in the list".to_string(),
                        };
                        ui.label(RichText::new(hint).color(tokens.text));
                        ui.horizontal(|ui| {
                            let here = fm_engine::picker_cmds::choosable(&picker.folder);
                            let label = if folder_kind {
                                "Choose folder"
                            } else {
                                "Choose file"
                            };
                            let ready = if folder_kind {
                                here
                            } else {
                                picker.selected.is_some()
                            };
                            if ui
                                .add_enabled_ui(ready, |ui| {
                                    sync_icons::button(ui, tokens, Icon::Check, label, true)
                                        .clicked()
                                })
                                .inner
                            {
                                accept = true;
                            }
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("Cancel").strong())
                                        .min_size(egui::vec2(100.0, 34.0)),
                                )
                                .on_hover_text("Close without choosing (Esc)")
                                .clicked()
                            {
                                cancel = true;
                            }
                        });
                    });
                });
            });
        });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        escape_used = true;
    }
    // Escape (or the row's Cancel) closes the new-folder row first, then the picker.
    if escape_used && draft.creating.take().is_none() {
        cancel = true;
    }
    if create.is_some() {
        draft.creating = None;
    }
    // A newly opened (or restored) chooser may sit under the dialog that asked for it.
    let shown_for = (picker.purpose.clone(), s.parked_pickers.len());
    let deeper = draft
        .shown_for
        .as_ref()
        .is_none_or(|(_, depth)| *depth < shown_for.1);
    let changed = draft.shown_for.as_ref() != Some(&shown_for);
    draft.shown_for = Some(shown_for);
    // Only a chooser opening on top is raised; one coming back leaves the dialog that
    // asked for the other chooser in front.
    if changed && deeper {
        ctx.move_to_top(egui::LayerId::new(layer, Id::NULL.with("fm-picker")));
        ctx.request_repaint();
    }
    ctx.data_mut(|d| d.insert_temp(draft_id(), draft));
    if cancel || !open {
        s.dispatch_or_notice("picker.cancel", &serde_json::Value::Null);
        return;
    }
    if let Some(name) = create {
        s.dispatch_or_notice("picker.mkdir", &json!({ "name": name }));
    }
    if let Some(uri) = go {
        s.dispatch_or_notice("picker.navigate", &json!({ "uri": uri.to_string() }));
    }
    if let Some(uri) = choose {
        s.dispatch_or_notice("picker.select", &json!({ "uri": uri.to_string() }));
    }
    if accept {
        s.dispatch_or_notice("picker.accept", &serde_json::Value::Null);
    }
}

/// The shortcuts column; returns the place to open.
fn places_column(
    ui: &mut egui::Ui,
    tokens: &Tokens,
    sections: &[(&'static str, Vec<fm_engine::places_cmds::Place>)],
    current: &VfsUri,
) -> Option<VfsUri> {
    let mut open = None;
    egui::ScrollArea::vertical()
        .id_salt("fm-picker-places")
        .max_height(390.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.spacing_mut().interact_size.y = 28.0;
            for (title, places) in sections {
                ui.label(RichText::new(*title).strong().color(tokens.text_muted));
                for place in places {
                    let r = ui
                        .horizontal(|ui| {
                            let (icon, _) = ui.allocate_exact_size(
                                egui::vec2(tokens.font_size + 6.0, 28.0),
                                egui::Sense::hover(),
                            );
                            crate::places::paint_icon(
                                ui.painter(),
                                place.kind,
                                icon.center(),
                                tokens.font_size * 0.6,
                                tokens.text_muted,
                            );
                            let button = ui.add_sized(
                                [ui.available_width(), 28.0],
                                egui::Button::selectable(place.uri == *current, "")
                                    .left_text(place.name.as_str())
                                    .truncate(),
                            );
                            button.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    place.name.as_str(),
                                )
                            });
                            button.on_hover_text(shown(&place.uri))
                        })
                        .inner;
                    if r.clicked() {
                        open = Some(place.uri.clone());
                    }
                }
                ui.add_space(tokens.padding * 0.5);
            }
        });
    open
}
