//! The unsaved-changes bar above a Files pane inside an archive with staged changes
//! (add-archive-vfs D8): "site.zip has 3 unsaved changes · Save · Discard", a list of
//! the changes on click, and an inline confirmation before Discard.

use egui::{Id, RichText, Ui};
use fm_engine::Session;
use fm_layout::PaneId;
use serde_json::json;

use crate::theme::Tokens;

fn plural(n: usize) -> &'static str {
    if n == 1 { "change" } else { "changes" }
}

fn confirm_id(pane: PaneId) -> Id {
    Id::NULL.with(("fm-archive-discard", pane))
}

/// Open the bar's Discard confirmation in `pane` (File › Archive › Discard).
pub fn ask_discard(ctx: &egui::Context, pane: PaneId) {
    ctx.data_mut(|d| d.insert_temp(confirm_id(pane), true));
}

pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) {
    let Some(location) = s.layout.state(pane).map(|p| p.location.clone()) else {
        return;
    };
    let Some(mount) = s.archives.for_uri(&location).cloned() else {
        return;
    };
    let Some(fs) = mount.opened().filter(|fs| fs.is_dirty()) else {
        return;
    };
    let root = mount.root().clone();
    let name = mount
        .container()
        .file_name()
        .unwrap_or("The archive")
        .to_string();
    let changes = fs.changes();
    let saving = fs.is_saving();
    let confirm_id = confirm_id(pane);
    let confirming = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(confirm_id))
        .unwrap_or(false);
    let expanded = s.archives.changes_shown.as_ref() == Some(&root);
    let root_s = root.to_string();
    let mut act: Option<(&str, serde_json::Value)> = None;
    egui::Frame::NONE
        .fill(tokens.panel)
        .stroke(egui::Stroke::new(tokens.border_width, tokens.warning))
        .corner_radius(tokens.corner_radius)
        .inner_margin(egui::Margin::symmetric(tokens.padding as i8, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                if confirming {
                    ui.label(
                        RichText::new(format!(
                            "Discard {} unsaved {} to {name}?",
                            changes.len(),
                            plural(changes.len())
                        ))
                        .color(tokens.text),
                    );
                    if ui.button("Discard").clicked() {
                        act = Some(("archive.discard", json!({"uri": root_s, "confirm": true})));
                        ui.ctx().data_mut(|d| d.remove::<bool>(confirm_id));
                    }
                    if ui.button("Keep").clicked() {
                        ui.ctx().data_mut(|d| d.remove::<bool>(confirm_id));
                    }
                    return;
                }
                let text = format!(
                    "{name} has {} unsaved {}",
                    changes.len(),
                    plural(changes.len())
                );
                let label = ui
                    .add(
                        egui::Label::new(RichText::new(text).color(tokens.text))
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(if expanded {
                        "Hide the list"
                    } else {
                        "Show what changed"
                    });
                if label.clicked() {
                    if expanded {
                        s.archives.changes_shown = None;
                    } else {
                        act = Some(("archive.changes", json!({"uri": root_s})));
                    }
                }
                if saving {
                    ui.label(RichText::new("Saving…").color(tokens.text_muted));
                    return;
                }
                if ui.button("Save").clicked() {
                    act = Some(("archive.save", json!({"uri": root_s})));
                }
                if ui
                    .button("Save as copy…")
                    .on_hover_text("Write the changed archive to another folder")
                    .clicked()
                {
                    act = Some(("archive.save_as", json!({"uri": root_s})));
                }
                if ui.button("Discard").clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(confirm_id, true));
                }
            });
            if expanded && !confirming {
                egui::ScrollArea::vertical()
                    .id_salt(("fm-archive-changes", pane))
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for c in &changes {
                            let what = match c.kind {
                                "added" => "Added",
                                "replaced" => "Replaced",
                                "folder" => "New folder",
                                _ => "Removed",
                            };
                            ui.label(
                                RichText::new(format!("{what}: {}", c.path))
                                    .color(tokens.text_muted),
                            );
                        }
                    });
            }
        });
    if let Some((id, params)) = act {
        let _ = s.layout.focus_pane(pane);
        s.dispatch_or_notice(id, &params);
    }
}
