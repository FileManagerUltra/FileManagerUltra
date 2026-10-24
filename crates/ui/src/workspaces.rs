//! Workspace tab bar and the saved-workspaces section of Preferences. Pure view over
//! `Session::workspaces`: every action is a `workspace.*` command.

use egui::{Rect, RichText, Sense, Ui};
use serde_json::{Value, json};

use fm_engine::Session;
use fm_engine::workspace::MAX_SAVED;

use crate::dialogs::{Dialog, Dialogs};
use crate::theme::Tokens;

type Action = (&'static str, Value);

fn flags_hint(s: &Session, uid: u64) -> String {
    match s.workspaces.saved_for(uid) {
        None => "Not saved".to_string(),
        Some(w) => {
            let mut parts = vec!["Saved"];
            if w.auto_save {
                parts.push("auto-save");
            }
            if w.open_at_startup {
                parts.push("opens at startup");
            }
            if w.default {
                parts.push("default");
            }
            parts.join(", ")
        }
    }
}

/// The row of workspace tabs: click to switch, `×` or middle-click to close, drag to
/// reorder, right-click for rename, save and the per-workspace options.
pub fn tab_bar(ui: &mut Ui, s: &mut Session, tokens: &Tokens, dialogs: &mut Dialogs) {
    let mut actions: Vec<Action> = Vec::new();
    let mut rename: Option<usize> = None;
    let mut rects: Vec<Rect> = Vec::new();
    let mut released: Option<usize> = None;
    let count = s.workspaces.tabs.len();
    egui::ScrollArea::horizontal()
        .id_salt("workspace-tabs")
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = tokens.padding;
                for ix in 0..count {
                    let Some(tab) = s.workspaces.tabs.get(ix) else {
                        continue;
                    };
                    let active = ix == s.workspaces.active;
                    let mut label = if ix < 10 {
                        format!("{} {}", (ix + 1) % 10, tab.name)
                    } else {
                        tab.name.clone()
                    };
                    if tab.dirty {
                        label.push_str(" •");
                    }
                    let hint = flags_hint(s, tab.uid);
                    let uid = tab.uid;
                    let resp = ui
                        .add(egui::Button::selectable(active, label).sense(Sense::click_and_drag()))
                        .on_hover_text(hint);
                    rects.push(resp.rect);
                    if resp.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if resp.drag_stopped() {
                        released = Some(ix);
                    } else if resp.clicked() && !active {
                        actions.push(("workspace.switch", json!({ "n": ix + 1 })));
                    }
                    if resp.middle_clicked() {
                        actions.push(("workspace.close", json!({ "tab": ix })));
                    }
                    resp.context_menu(|ui| {
                        let saved = s.workspaces.saved_for(uid);
                        if ui.button("Rename…").clicked() {
                            rename = Some(ix);
                            ui.close();
                        }
                        if ui.button("Save").clicked() {
                            actions.push(("workspace.save", json!({ "tab": ix })));
                            ui.close();
                        }
                        ui.separator();
                        let flags = [
                            (
                                "Auto-save",
                                "workspace.autosave",
                                saved.is_some_and(|w| w.auto_save),
                            ),
                            (
                                "Open at startup",
                                "workspace.startup",
                                saved.is_some_and(|w| w.open_at_startup),
                            ),
                            (
                                "Default workspace",
                                "workspace.default",
                                saved.is_some_and(|w| w.default),
                            ),
                        ];
                        for (text, id, mut on) in flags {
                            if ui.checkbox(&mut on, text).changed() {
                                actions.push((id, json!({ "tab": ix, "on": on })));
                            }
                        }
                        ui.separator();
                        if ui.button("Close").clicked() {
                            actions.push(("workspace.close", json!({ "tab": ix })));
                            ui.close();
                        }
                    });
                    let close = ui
                        .add(egui::Button::new(tokens.muted("×")).frame(false))
                        .on_hover_text("Close workspace");
                    if close.clicked() {
                        actions.push(("workspace.close", json!({ "tab": ix })));
                    }
                }
                if ui
                    .add(egui::Button::new("+"))
                    .on_hover_text("New workspace (Ctrl+T)")
                    .clicked()
                {
                    actions.push(("workspace.new", json!({})));
                }
            });
        });
    if let Some(from) = released
        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
        && let Some(to) = rects
            .iter()
            .position(|r| pos.x >= r.left() && pos.x <= r.right())
        && to != from
    {
        actions.push(("workspace.move", json!({ "from": from, "to": to })));
    }
    for (id, params) in actions {
        s.dispatch_or_notice(id, &params);
    }
    if let Some(ix) = rename
        && let Some(tab) = s.workspaces.tabs.get(ix)
    {
        dialogs.request(Dialog::RenameWorkspace {
            tab: ix,
            name: tab.name.clone(),
        });
    }
}

/// Preferences: the saved workspaces and their startup behaviour.
pub fn prefs_section(ui: &mut Ui, s: &mut Session, tokens: &Tokens) {
    ui.label(RichText::new("Workspaces").color(tokens.accent).strong());
    if s.workspaces.saved.is_empty() {
        ui.label(tokens.muted("No saved workspaces (Workspace ▸ Save)."));
        return;
    }
    let mut actions: Vec<Action> = Vec::new();
    egui::Grid::new("fm-prefs-workspaces")
        .num_columns(5)
        .spacing([tokens.padding * 2.0, tokens.padding])
        .show(ui, |ui| {
            for h in ["Name", "Auto-save", "At startup", "Default", ""] {
                ui.label(tokens.muted(h));
            }
            ui.end_row();
            for w in &s.workspaces.saved {
                ui.label(&w.name);
                for (id, mut on) in [
                    ("workspace.autosave", w.auto_save),
                    ("workspace.startup", w.open_at_startup),
                    ("workspace.default", w.default),
                ] {
                    if ui.checkbox(&mut on, "").changed() {
                        actions.push((id, json!({ "name": w.name, "on": on })));
                    }
                }
                if ui.small_button("Delete").clicked() {
                    actions.push(("workspace.delete", json!({ "name": w.name })));
                }
                ui.end_row();
            }
        });
    ui.label(tokens.muted(format!(
        "{} of {MAX_SAVED} saved. With none marked for startup, the default workspace opens.",
        s.workspaces.saved.len()
    )));
    for (id, params) in actions {
        s.dispatch_or_notice(id, &params);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn frame(s: &mut Session, tokens: &Tokens) {
        let ctx = egui::Context::default();
        crate::theme::apply_to(&ctx, tokens);
        let mut dialogs = Dialogs::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                tab_bar(ui, s, tokens, &mut dialogs);
                prefs_section(ui, s, tokens);
            });
        });
        out.textures_delta.clear();
    }

    #[test]
    fn the_tab_bar_and_preferences_render_in_both_palettes() {
        let mut s = Session::new().expect("session");
        for name in ["Work", "Media"] {
            s.dispatch("workspace.new", &json!({ "name": name }))
                .expect("new");
            s.dispatch("workspace.save", &json!({})).expect("save");
        }
        s.dispatch("workspace.autosave", &json!({})).expect("auto");
        for tokens in [Tokens::dark(), Tokens::light()] {
            frame(&mut s, &tokens);
        }
        assert_eq!(s.workspaces.tabs.len(), 3);
    }

    #[test]
    fn many_tabs_do_not_panic() {
        let mut s = Session::new().expect("session");
        for _ in 0..25 {
            s.dispatch("workspace.new", &json!({})).expect("new");
        }
        frame(&mut s, &Tokens::dark());
    }
}
