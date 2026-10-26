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
            ui.label(RichText::new("Preview").color(tokens.accent).strong());
            let mut recursive = s.prefs.recursive_preview;
            let response = ui
                .checkbox(&mut recursive, "Recursive folder preview")
                .on_hover_text(
                    "A folder in the preview pane counts, and in thumbnail mode lists, \
                     every file below it. Bounded by entry, depth and time limits; \
                     links are never followed.",
                );
            if response.changed() {
                s.dispatch_or_notice("prefs.preview.recursive", &json!({"value": recursive}));
            }
            ui.add_space(tokens.padding);
            ui.label(RichText::new("SFTP").color(tokens.accent).strong());
            let mut poll = s.prefs.sftp_poll_secs;
            let mut timeout = s.prefs.sftp_timeout_secs;
            let mut keepalive = s.prefs.sftp_keepalive_secs;
            let mut changed = false;
            egui::Grid::new("fm-prefs-sftp").num_columns(2).show(ui, |ui| {
                for (label, value, range, tip) in [
                    (
                        "Refresh interval (s)",
                        &mut poll,
                        fm_engine::SFTP_POLL_RANGE,
                        "How often connected remote panes re-read their folder; a dead link \
                         is noticed at the next refresh.",
                    ),
                    (
                        "Stalled call timeout (s)",
                        &mut timeout,
                        fm_engine::SFTP_TIMEOUT_RANGE,
                        "A remote call that makes no progress for this long marks the \
                         connection offline.",
                    ),
                    (
                        "Keepalive interval (s)",
                        &mut keepalive,
                        fm_engine::SFTP_KEEPALIVE_RANGE,
                        "Idle links are probed this often; three misses drop the link. \
                         Applies to new connections.",
                    ),
                ] {
                    let caption = ui.label(label);
                    changed |= ui
                        .add(egui::DragValue::new(value).range(range).speed(0.2))
                        .on_hover_text(tip)
                        .labelled_by(caption.id)
                        .changed();
                    ui.end_row();
                }
            });
            if changed {
                s.dispatch_or_notice(
                    "prefs.sftp.timing",
                    &json!({"poll_secs": poll, "timeout_secs": timeout, "keepalive_secs": keepalive}),
                );
            }
            let hosts: Vec<String> = s
                .remote
                .known_hosts()
                .entries()
                .iter()
                .map(|h| h.host.clone())
                .collect();
            if !hosts.is_empty() {
                ui.add_space(tokens.padding);
                ui.label(tokens.muted("Trusted hosts (asked again after forgetting)"));
                for host in hosts {
                    ui.horizontal(|ui| {
                        ui.label(&host);
                        if ui.button("Forget").clicked() {
                            s.dispatch_or_notice("remote.hostkey.forget", &json!({"host": host}));
                        }
                    });
                }
            }
            ui.add_space(tokens.padding);
            crate::workspaces::prefs_section(ui, s, tokens);
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
