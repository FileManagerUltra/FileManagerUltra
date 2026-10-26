//! The right-click menu shared by the Files listing and the thumbnail grid.
//!
//! Entries come from the command registry (label, shortcut, `enabled`), so the menu never
//! duplicates command knowledge. The widget only reads the session: a chosen command id is
//! queued on the egui context and the app dispatches it (dialogs included) after the frame.

use egui::{Context, Id, Response, Ui};
use fm_engine::Session;
use fm_vfs::VfsUri;

/// Command ids in menu order; `None` is a separator.
const ENTRIES: &[Option<&str>] = &[
    Some("file.clip_cut"),
    Some("file.clip_copy"),
    Some("file.clip_paste"),
    None,
    Some("file.copy"),
    Some("file.move"),
    Some("file.rename"),
    None,
    Some("file.mkdir"),
    Some("file.newfile"),
    None,
    Some("file.delete"),
    Some("file.delete_permanent"),
];

fn queue_id() -> Id {
    Id::NULL.with("context-menu-command")
}

/// Attach the item menu to `response` (the row or thumbnail of `uri`). The caller selects the
/// item on right-click first, so the commands act on what the user pointed at. "Open" is
/// reported through `open`, like a double click.
pub fn attach(response: &Response, s: &Session, uri: &VfsUri, open: &mut Option<VfsUri>) {
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            *open = Some(uri.clone());
            ui.close();
        }
        for entry in ENTRIES {
            match entry {
                None => {
                    ui.separator();
                }
                Some(id) => command_item(ui, s, id),
            }
        }
    });
}

/// The menu of a saved-server row (Saved servers folder): connect, edit, delete. These act on
/// the profile called `name`, so they dispatch straight away with that parameter.
pub fn attach_server(
    response: &Response,
    s: &mut Session,
    name: &str,
    uri: &VfsUri,
    open: &mut Option<VfsUri>,
) {
    response.context_menu(|ui| {
        if ui.button("Connect").clicked() {
            *open = Some(uri.clone());
            ui.close();
        }
        if ui.button("Edit…").clicked() {
            s.dispatch_or_notice("servers.edit", &serde_json::json!({"name": name}));
            ui.close();
        }
        if ui.button("Delete").clicked() {
            s.dispatch_or_notice("remote.profile.delete", &serde_json::json!({"name": name}));
            ui.close();
        }
    });
}

fn command_item(ui: &mut Ui, s: &Session, id: &str) {
    let Some(spec) = s.registry.get(id) else {
        return;
    };
    let mut button = egui::Button::new(spec.label);
    if let Some(shortcut) = spec.shortcut {
        button = button.shortcut_text(shortcut);
    }
    if ui.add_enabled((spec.enabled)(s), button).clicked() {
        ui.ctx()
            .data_mut(|d| d.insert_temp::<&'static str>(queue_id(), spec.id));
        ui.close();
    }
}

/// The command chosen in a context menu this frame, if any.
pub fn take_command(ctx: &Context) -> Option<&'static str> {
    ctx.data_mut(|d| d.remove_temp::<&'static str>(queue_id()))
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_a_registered_command() {
        let s = Session::new().expect("session");
        for id in ENTRIES.iter().flatten() {
            assert!(s.registry.contains(id), "dead context-menu id: {id}");
        }
    }

    #[test]
    fn a_chosen_command_is_taken_exactly_once() {
        let ctx = Context::default();
        ctx.data_mut(|d| d.insert_temp::<&'static str>(queue_id(), "file.copy"));
        assert_eq!(take_command(&ctx), Some("file.copy"));
        assert_eq!(take_command(&ctx), None);
    }
}
