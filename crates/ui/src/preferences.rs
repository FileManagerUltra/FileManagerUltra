//! The Preferences dialog. Pure view over `Session::prefs`: every change is a command
//! (`view.theme.*`, `prefs.links.retarget`), so the UI never edits settings itself.

use egui::{Context, Id, Key, RichText};
use serde_json::json;

use fm_engine::{Params, Session, ThemePreference};

use crate::dialogs::frame;
use crate::theme::Tokens;

/// Render the dialog while `Session::preferences_open`; Esc or the button closes it.
pub fn show(ctx: &Context, s: &mut Session, tokens: &Tokens) {
    if !s.preferences_open {
        return;
    }
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    egui::Window::new("Preferences")
        .id(Id::NULL.with("fm-preferences"))
        .collapsible(false)
        .movable(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .frame(frame(tokens))
        .show(ctx, |ui| {
            ui.label(RichText::new("Appearance").color(tokens.accent).strong());
            ui.horizontal(|ui| {
                ui.label("Theme");
                for (label, id, value) in [
                    ("System", "view.theme.system", ThemePreference::System),
                    ("Dark", "view.theme.dark", ThemePreference::Dark),
                    ("Light", "view.theme.light", ThemePreference::Light),
                ] {
                    if ui.selectable_label(s.prefs.theme == value, label).clicked() {
                        s.dispatch_or_notice(id, &Params::Null);
                    }
                }
            });
            ui.add_space(tokens.padding);
            ui.label(RichText::new("Copy and move").color(tokens.accent).strong());
            let mut retarget = s.prefs.retarget_links;
            let response = ui
                .checkbox(&mut retarget, "Re-target links when copying")
                .on_hover_text(
                    "Absolute symbolic links that point inside the copied folder are \
                     re-pointed at the copy. Relative links and links to other places \
                     are copied as they are. Links are never followed.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.links.retarget", &json!({"value": retarget}));
            }
            ui.add_space(tokens.padding);
            ui.label(
                RichText::new("Changes apply to new jobs and are saved on exit.")
                    .color(tokens.text_muted),
            );
            ui.add_space(tokens.padding);
            if ui.button("Close").clicked() {
                close = true;
            }
        });
    if close {
        s.preferences_open = false;
    }
}
