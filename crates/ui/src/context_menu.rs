//! The right-click menu shared by the Files listing and the thumbnail grid.
//!
//! Entries come from the command registry (label, shortcut, `enabled`), so the menu never
//! duplicates command labels. A chosen command is queued on the egui context and the app
//! dispatches it (dialogs included) after the frame.

use egui::{Context, Id, Response, Ui};
use fm_engine::Session;
use fm_layout::PaneId;
use fm_vfs::VfsUri;

const FOLDER_ENTRIES: &[Option<&str>] = &[
    Some("file.mkdir"),
    Some("file.newfile"),
    None,
    Some("file.clip_paste"),
    None,
    Some("nav.refresh"),
    Some("file.copy_path"),
    Some("file.copy_name"),
    None,
    Some("bookmark.add"),
    Some("file.properties"),
];

/// Command ids in menu order; `None` is a separator.
const ENTRIES: &[Option<&str>] = &[
    Some("file.open_with.dialog"),
    None,
    Some("file.clip_cut"),
    Some("file.clip_copy"),
    Some("file.clip_paste"),
    None,
    Some("file.copy"),
    Some("file.move"),
    Some("file.rename"),
    Some("file.duplicate"),
    None,
    Some("file.copy_path"),
    Some("file.copy_name"),
    None,
    Some("file.mkdir"),
    Some("file.newfile"),
    None,
    Some("diff.compare.head"),
    Some("diff.mark"),
    Some("diff.compare.marked"),
    Some("diff.compare.selected"),
    None,
    Some("file.delete"),
    Some("file.delete_permanent"),
    None,
    Some("bookmark.add"),
    Some("file.properties"),
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
        if let Some(spec) = s.registry.get("preview.open")
            && ui
                .add_enabled((spec.enabled)(s), egui::Button::new(spec.label))
                .clicked()
        {
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    Id::NULL.with("folder-context-command"),
                    Some(FolderAction {
                        id: spec.id,
                        folder: uri.clone(),
                    }),
                )
            });
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

/// A context command retains its explicit URI target until dispatch, including for dialogs.
#[derive(Clone)]
pub struct FolderAction {
    pub id: &'static str,
    pub folder: VfsUri,
}

pub fn take_folder_action(ctx: &Context) -> Option<FolderAction> {
    ctx.data_mut(|d| {
        d.remove_temp::<Option<FolderAction>>(Id::NULL.with("folder-context-command"))
            .flatten()
    })
}

/// Blank listing space uses the current folder; tree rows supply their own folder.
pub fn attach_folder(response: &Response, s: &mut Session, pane: PaneId, folder: Option<&VfsUri>) {
    if response.secondary_clicked() {
        if folder.is_some() {
            response.request_focus();
        } else {
            response.ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        if let Err(err) = s.layout.focus_pane(pane) {
            s.notice = Some(err.to_string());
            return;
        }
        if let Some(state) = s.layout.state_mut(pane) {
            if let Some(folder) = folder {
                state.tree.active = true;
                state.tree.focused = Some(folder.clone());
            } else {
                state.tree.active = false;
                state.listing.focus_current();
            }
        }
    }
    let Some(folder) = folder
        .cloned()
        .or_else(|| s.layout.state(pane).map(|p| p.location.clone()))
    else {
        return;
    };
    response.context_menu(|ui| {
        for entry in FOLDER_ENTRIES {
            match entry {
                Some(id) => {
                    let Some(spec) = s.registry.get(id) else {
                        continue;
                    };
                    let mut button = egui::Button::new(spec.label);
                    if let Some(shortcut) = spec.shortcut {
                        button = button.shortcut_text(shortcut);
                    }
                    let virtual_folder = folder.is_drive_list()
                        || matches!(
                            folder.scheme(),
                            fm_vfs::Scheme::Network | fm_vfs::Scheme::Servers
                        );
                    let enabled = (spec.enabled)(s)
                        && !(virtual_folder
                            && matches!(*id, "file.mkdir" | "file.newfile" | "file.clip_paste"));
                    if ui.add_enabled(enabled, button).clicked() {
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(
                                Id::NULL.with("folder-context-command"),
                                Some(FolderAction {
                                    id: spec.id,
                                    folder: folder.clone(),
                                }),
                            )
                        });
                        ui.close();
                    }
                }
                None => {
                    ui.separator();
                }
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
    use egui_kittest::{Harness, kittest::Queryable};
    use fm_testkit::MemFs;
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    fn folder_session(empty: bool) -> Session {
        let mut s = Session::new().expect("session");
        let mem = MemFs::new();
        mem.mkdirs_str("/d").expect("folder");
        if !empty {
            mem.mkdirs_str("/d/sub").expect("child folder");
            mem.put_str("/d/file.txt", b"test").expect("file");
        }
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/d").expect("uri");
        let state = s.layout.state_mut(1).expect("pane");
        state.location = uri.clone();
        state.tree = fm_layout::TreeState::new(uri.clone());
        state.listing.select_only(Some("file.txt"), 0);
        s.ensure_read(uri.clone()).expect("read");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !s.dirs.state(&uri).is_ready() {
            assert!(Instant::now() < deadline);
            s.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        s
    }

    fn secondary_click(harness: &mut Harness<(Session, Option<FolderAction>)>, pos: egui::Pos2) {
        for pressed in [true, false] {
            harness.input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            harness.step();
        }
        harness.run();
    }

    #[test]
    fn listing_background_menu_clears_selection_and_targets_its_own_pane() {
        for tokens in [crate::theme::Tokens::dark(), crate::theme::Tokens::light()] {
            for (empty, filtered) in [(true, false), (false, false), (false, true)] {
                let mut s = folder_session(empty);
                if filtered {
                    s.layout.state_mut(1).expect("pane").listing.filter = Some("no matches".into());
                    s.invalidate_view(1);
                }
                let other = s
                    .layout
                    .split(1, fm_layout::Axis::Horizontal, 0.5)
                    .expect("split");
                s.layout.focus_pane(other).expect("focus other");
                let mut harness = Harness::builder()
                    .with_size(egui::vec2(700.0, 500.0))
                    .build_ui_state(
                        move |ui, (s, action)| {
                            crate::theme::apply_to(ui.ctx(), &tokens);
                            crate::listing::show(ui, s, 1, &tokens, s.layout.focused() == 1);
                            if let Some(chosen) = take_folder_action(ui.ctx()) {
                                *action = Some(chosen);
                            }
                        },
                        (s, None),
                    );
                secondary_click(&mut harness, egui::pos2(300.0, 350.0));
                assert_eq!(harness.state().0.layout.focused(), 1);
                let pane = harness.state().0.layout.state(1).expect("pane");
                assert!(pane.listing.selection().next().is_none());
                assert!(pane.listing.on_current);
                assert!(!pane.tree.active);
                assert!(
                    harness.query_by_label("Open").is_none(),
                    "must not show the item menu"
                );
                harness.get_by_label("New Folder F7").click();
                harness.run();
                let action = harness.state().1.as_ref().expect("queued command");
                assert_eq!(action.id, "file.mkdir");
                assert_eq!(action.folder, VfsUri::parse("/d").expect("uri"));
            }
        }
    }

    #[test]
    fn tree_menu_targets_clicked_folder_without_navigating() {
        for tokens in [crate::theme::Tokens::dark(), crate::theme::Tokens::light()] {
            let mut s = folder_session(false);
            let other = s
                .layout
                .split(1, fm_layout::Axis::Horizontal, 0.5)
                .expect("split");
            s.layout.focus_pane(other).expect("focus other");
            let mut harness = Harness::builder()
                .with_size(egui::vec2(700.0, 500.0))
                .build_ui_state(
                    move |ui, (s, action)| {
                        crate::theme::apply_to(ui.ctx(), &tokens);
                        assert!(crate::tree::show(ui, s, 1, &tokens).is_none());
                        if let Some(chosen) = take_folder_action(ui.ctx()) {
                            *action = Some(chosen);
                        }
                    },
                    (s, None),
                );
            harness.run();
            let pos = harness.get_by_label("   sub").rect().center();
            secondary_click(&mut harness, pos);
            assert_eq!(harness.state().0.layout.focused(), 1);
            let pane = harness.state().0.layout.state(1).expect("pane");
            assert!(
                pane.tree.active,
                "position={pos:?}, focused={:?}",
                pane.tree.focused
            );
            assert_eq!(
                pane.tree.focused,
                Some(VfsUri::parse("/d/sub").expect("uri"))
            );
            assert_eq!(pane.location, VfsUri::parse("/d").expect("uri"));
            assert!(
                !pane
                    .tree
                    .is_expanded(&VfsUri::parse("/d/sub").expect("uri"))
            );
            harness.get_by_label("New File Shift+F7").click();
            harness.run();
            let action = harness.state().1.as_ref().expect("queued command");
            assert_eq!(action.id, "file.newfile");
            assert_eq!(action.folder, VfsUri::parse("/d/sub").expect("uri"));
        }
    }

    #[test]
    fn listing_rows_keep_the_item_menu_instead_of_the_background_menu() {
        let tokens = crate::theme::Tokens::dark();
        let s = folder_session(false);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 500.0))
            .build_ui_state(
                move |ui, (s, _action)| {
                    crate::theme::apply_to(ui.ctx(), &tokens);
                    crate::listing::show(ui, s, 1, &tokens, true);
                },
                (s, None),
            );
        harness.run();
        let pos = harness.get_by_label("Current folder").rect().center()
            + egui::vec2(
                0.0,
                2.0 * (tokens.row_height
                    + harness
                        .ctx
                        .style_of(egui::Theme::Dark)
                        .spacing
                        .item_spacing
                        .y),
            );
        secondary_click(&mut harness, pos);
        assert!(harness.query_by_label("Open").is_some());
        assert!(
            harness
                .state()
                .0
                .layout
                .state(1)
                .expect("pane")
                .listing
                .is_selected("sub")
        );
    }

    #[test]
    fn file_context_preview_action_targets_the_right_clicked_file_in_an_unfocused_pane() {
        for tokens in [crate::theme::Tokens::dark(), crate::theme::Tokens::light()] {
            let mut s = folder_session(false);
            let other = s
                .layout
                .split(1, fm_layout::Axis::Horizontal, 0.5)
                .expect("split");
            s.layout.focus_pane(other).expect("focus");
            let mut harness = Harness::builder()
                .with_size(egui::vec2(700.0, 500.0))
                .build_ui_state(
                    move |ui, (s, action)| {
                        crate::theme::apply_to(ui.ctx(), &tokens);
                        crate::listing::show(ui, s, 1, &tokens, s.layout.focused() == 1);
                        if let Some(chosen) = take_folder_action(ui.ctx()) {
                            *action = Some(chosen);
                        }
                    },
                    (s, None),
                );
            harness.run();
            let pos = harness.get_by_label("Current folder").rect().center()
                + egui::vec2(
                    0.0,
                    3.0 * (tokens.row_height
                        + harness
                            .ctx
                            .style_of(egui::Theme::Dark)
                            .spacing
                            .item_spacing
                            .y),
                );
            secondary_click(&mut harness, pos);
            assert_eq!(harness.state().0.layout.focused(), 1);
            harness.get_by_label("Open in Preview Panel").click();
            harness.run();
            let action = harness.state().1.as_ref().expect("preview action");
            assert_eq!(action.id, "preview.open");
            assert_eq!(action.folder, VfsUri::parse("/d/file.txt").expect("uri"));
            let params = serde_json::json!({"uri": action.folder.to_string()});
            harness
                .state_mut()
                .0
                .dispatch("preview.open", &params)
                .expect("open preview");
            assert_eq!(harness.state().0.layout.panes().len(), 3);
            harness
                .state_mut()
                .0
                .dispatch("preview.open", &params)
                .expect("reuse preview");
            assert_eq!(harness.state().0.layout.panes().len(), 3);
        }
    }

    #[test]
    fn every_entry_is_a_registered_command() {
        let s = Session::new().expect("session");
        for id in ENTRIES.iter().chain(FOLDER_ENTRIES).flatten() {
            assert!(s.registry.contains(id), "dead context-menu id: {id}");
        }
        assert!(s.registry.contains("preview.open"));
    }

    #[test]
    fn a_chosen_command_is_taken_exactly_once() {
        let ctx = Context::default();
        ctx.data_mut(|d| d.insert_temp::<&'static str>(queue_id(), "file.copy"));
        assert_eq!(take_command(&ctx), Some("file.copy"));
        assert_eq!(take_command(&ctx), None);
        ctx.data_mut(|d| {
            d.insert_temp(
                Id::NULL.with("folder-context-command"),
                Some(FolderAction {
                    id: "file.mkdir",
                    folder: VfsUri::parse("/d").expect("uri"),
                }),
            )
        });
        assert_eq!(take_folder_action(&ctx).expect("action").id, "file.mkdir");
        assert!(take_folder_action(&ctx).is_none());
    }
}
