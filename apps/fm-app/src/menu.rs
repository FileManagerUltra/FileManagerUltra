//! Menu catalogue and keyboard shortcuts.
//!
//! Every id referenced here MUST exist in the engine registry — asserted by the tests
//! in this crate and by `cargo xtask brand-check`. The UI never calls operations
//! directly: it only dispatches command ids.

use eframe::egui;

use fm_engine::{Params, Session, ThemePreference};
use fm_layout::PaneKind;
use fm_ui::dialogs::{Dialogs, dialog_for};

pub const PREFERENCES: &str = "app.preferences";
pub const QUIT: &str = "app.quit";
pub const ABOUT: &str = "app.about";
pub const CHECK_UPDATES: &str = "app.updates.check";
pub const THEME_DARK: &str = "view.theme.dark";
pub const THEME_LIGHT: &str = "view.theme.light";
pub const THEME_SYSTEM: &str = "view.theme.system";
pub const THEME_TOGGLE: &str = "view.theme.toggle";
pub const WINDOW_FULLSCREEN: &str = "window.fullscreen";
pub const PANE_SPLIT_H: &str = "pane.split.horizontal";
pub const PANE_SPLIT_V: &str = "pane.split.vertical";
pub const PANE_CLOSE: &str = "pane.close";
pub const PANE_SWAP: &str = "pane.swap";
pub const PANE_EQUALIZE: &str = "pane.equalize";
pub const PANE_ZOOM: &str = "pane.zoom";
pub const ZOOM_IN: &str = "view.zoom.in";
pub const ZOOM_OUT: &str = "view.zoom.out";
pub const ZOOM_RESET: &str = "view.zoom.reset";
pub const PANE_ROTATE: &str = "pane.rotate";
pub const PANE_RESIZE_LEFT: &str = "pane.resize.left";
pub const PANE_RESIZE_RIGHT: &str = "pane.resize.right";
pub const PANE_RESIZE_UP: &str = "pane.resize.up";
pub const PANE_RESIZE_DOWN: &str = "pane.resize.down";
pub const TERM_COPY: &str = "term.copy";
pub const TERM_PASTE: &str = "term.paste";
pub const TERM_CD_HERE: &str = "term.cd_here";
pub const TERM_CLEAR: &str = "term.clear_scrollback";
pub const VIEW_PREVIEW: &str = "view.preview.toggle";
pub const PREVIEW_PRETTY: &str = "preview.toggle_pretty";
pub const PREVIEW_SOURCE: &str = "preview.toggle_source";
pub const PREVIEW_WRAP: &str = "preview.toggle_wrap";
pub const PREVIEW_SELECT_ALL: &str = "preview.text.select_all";
pub const PREVIEW_COPY_TEXT: &str = "preview.text.copy";
pub const PREVIEW_FIND: &str = "preview.find.open";
pub const PREVIEW_SCALE_IN: &str = "preview.scale.in";
pub const PREVIEW_SCALE_OUT: &str = "preview.scale.out";
pub const PREVIEW_SCALE_RESET: &str = "preview.scale.reset";
pub const DIFF_NEXT: &str = "preview.diff.next";
pub const DIFF_PREV: &str = "preview.diff.prev";
pub const LOG_FOLLOW: &str = "preview.log.follow";
pub const LOG_COLUMNS: &str = "preview.log.columns";
pub const LOG_MARK: &str = "preview.log.mark";
pub const THUMBS_TOGGLE: &str = "preview.thumbs.toggle";
pub const THUMBS_SIZE: &str = "preview.thumbs.size";
pub const THUMBS_CLEAR: &str = "preview.thumbs.clear_cache";
pub const VIDEO_PLAY: &str = "preview.video.play";
pub const VIDEO_MUTE: &str = "preview.video.mute";
pub const AUDIO_PLAY: &str = "preview.audio.play";
pub const AUDIO_MUTE: &str = "preview.audio.mute";
pub const LOG_EARLIER: &str = "preview.log.earlier";
pub const TERM_RESTART: &str = "term.restart";
pub const PANE_FOCUS_NEXT: &str = "pane.focus.next";
pub const NAV_UP: &str = "nav.up";
pub const NAV_BACK: &str = "nav.back";
pub const NAV_FORWARD: &str = "nav.forward";
pub const NAV_REFRESH: &str = "nav.refresh";
pub const NAV_GOTO: &str = "nav.goto";
pub const SORT_NAME: &str = "view.sort.name";
pub const SORT_SIZE: &str = "view.sort.size";
pub const SORT_MTIME: &str = "view.sort.mtime";
pub const SORT_TYPE: &str = "view.sort.type";
pub const VIEW_HIDDEN: &str = "view.hidden.toggle";
pub const VIEW_LINKS: &str = "view.links.toggle";
pub const SORT_DIRS_FIRST: &str = "view.sort.dirs_first";
pub const FILE_COPY: &str = "file.copy";
pub const FILE_MOVE: &str = "file.move";
pub const FILE_MKDIR: &str = "file.mkdir";
pub const FILE_NEWFILE: &str = "file.newfile";
pub const FILE_RENAME: &str = "file.rename";
pub const REMOTE_CONNECT: &str = "remote.connect.dialog";
pub const NETWORK_CONNECT: &str = "network.connect.dialog";
pub const NETWORK_OPEN: &str = "network.open";
pub const REMOTE_RECONNECT: &str = "remote.reconnect";
pub const REMOTE_DISCONNECT: &str = "remote.disconnect";
pub const FILE_DELETE: &str = "file.delete";
pub const FILE_DELETE_PERMANENT: &str = "file.delete_permanent";
pub const TRASH_OPEN: &str = "trash.open";
pub const EDIT_UNDO: &str = "edit.undo";
pub const FILE_CLIP_COPY: &str = "file.clip_copy";
pub const FILE_CLIP_CUT: &str = "file.clip_cut";
pub const FILE_CLIP_PASTE: &str = "file.clip_paste";
pub const WS_NEW: &str = "workspace.new";
pub const WS_CLOSE: &str = "workspace.close";
pub const WS_SAVE: &str = "workspace.save";
pub const WS_RENAME: &str = "workspace.rename";
pub const SELECT_ALL: &str = "edit.select_all";
pub const SELECT_NONE: &str = "edit.select_none";
pub const SELECT_INVERT: &str = "edit.invert_selection";
pub const SELECT_PATTERN: &str = "edit.select_pattern";
pub const VIEW_FILTER: &str = "view.filter";
pub const VIEW_FILTER_CLEAR: &str = "view.filter.clear";
pub const COPY_PATH: &str = "file.copy_path";
pub const IMAGE_COPY: &str = "image.copy";
pub const COPY_NAME: &str = "file.copy_name";
pub const FILE_DUPLICATE: &str = "file.duplicate";
pub const OPEN_WITH: &str = "file.open_with.dialog";
pub const COMPARE_WITH: &str = "file.compare_with";
pub const PROPERTIES: &str = "file.properties";
pub const FIND_FILES: &str = "search.open";
pub const BOOKMARK_ADD: &str = "bookmark.add";

/// Every command id referenced by the menus or the shortcut table.
/// Used by the tests; brand-check separately scans command literals in this file.
#[cfg(test)]
pub fn all_ids() -> &'static [&'static str] {
    &[
        QUIT,
        ABOUT,
        CHECK_UPDATES,
        PREFERENCES,
        THEME_DARK,
        THEME_LIGHT,
        THEME_SYSTEM,
        THEME_TOGGLE,
        WINDOW_FULLSCREEN,
        PANE_SPLIT_H,
        PANE_SPLIT_V,
        PANE_CLOSE,
        PANE_SWAP,
        PANE_EQUALIZE,
        PANE_ZOOM,
        ZOOM_IN,
        ZOOM_OUT,
        ZOOM_RESET,
        PANE_ROTATE,
        PANE_RESIZE_LEFT,
        PANE_RESIZE_RIGHT,
        PANE_RESIZE_UP,
        PANE_RESIZE_DOWN,
        PANE_FOCUS_NEXT,
        TERM_COPY,
        TERM_PASTE,
        TERM_CD_HERE,
        TERM_CLEAR,
        TERM_RESTART,
        VIEW_PREVIEW,
        PREVIEW_PRETTY,
        PREVIEW_SOURCE,
        PREVIEW_WRAP,
        PREVIEW_SELECT_ALL,
        PREVIEW_COPY_TEXT,
        PREVIEW_SCALE_IN,
        PREVIEW_SCALE_OUT,
        PREVIEW_SCALE_RESET,
        LOG_FOLLOW,
        LOG_COLUMNS,
        LOG_MARK,
        LOG_EARLIER,
        THUMBS_TOGGLE,
        THUMBS_SIZE,
        THUMBS_CLEAR,
        VIDEO_PLAY,
        VIDEO_MUTE,
        AUDIO_PLAY,
        AUDIO_MUTE,
        "media.play",
        "media.enqueue",
        "media.play_next",
        "media.play_folder",
        "media.toggle",
        "media.stop",
        "media.next",
        "media.prev",
        "media.step",
        "media.speed",
        "media.eq",
        "media.loop",
        "media.sleep",
        "media.fullscreen",
        "media.mute",
        "media.shuffle",
        "media.clear",
        "media.playlist.load",
        "media.playlist.save",
        NAV_UP,
        NAV_BACK,
        NAV_FORWARD,
        NAV_REFRESH,
        NAV_GOTO,
        SORT_NAME,
        SORT_SIZE,
        SORT_MTIME,
        SORT_TYPE,
        SORT_DIRS_FIRST,
        VIEW_HIDDEN,
        VIEW_LINKS,
        FILE_COPY,
        FILE_MOVE,
        FILE_MKDIR,
        FILE_NEWFILE,
        FILE_RENAME,
        REMOTE_CONNECT,
        NETWORK_CONNECT,
        NETWORK_OPEN,
        REMOTE_RECONNECT,
        REMOTE_DISCONNECT,
        FILE_DELETE,
        FILE_DELETE_PERMANENT,
        TRASH_OPEN,
        EDIT_UNDO,
        FILE_CLIP_COPY,
        FILE_CLIP_CUT,
        FILE_CLIP_PASTE,
        WS_NEW,
        WS_CLOSE,
        WS_SAVE,
        WS_RENAME,
        SELECT_ALL,
        SELECT_NONE,
        SELECT_INVERT,
        SELECT_PATTERN,
        VIEW_FILTER,
        VIEW_FILTER_CLEAR,
        COPY_PATH,
        COPY_NAME,
        IMAGE_COPY,
        FILE_DUPLICATE,
        OPEN_WITH,
        COMPARE_WITH,
        PROPERTIES,
        FIND_FILES,
        BOOKMARK_ADD,
    ]
}

/// Keyboard shortcuts, id → egui binding. Display strings live on the CommandSpecs
/// (`"Ctrl+Q"`, `"Alt+Left"`…); the test in this module checks they agree.
pub fn shortcuts() -> Vec<(&'static str, egui::KeyboardShortcut)> {
    use egui::{Key, Modifiers};
    vec![
        (
            PREFERENCES,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Comma),
        ),
        (
            WS_NEW,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::T),
        ),
        (
            WS_CLOSE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::W),
        ),
        (
            QUIT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Q),
        ),
        (
            THEME_TOGGLE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::T),
        ),
        (
            WINDOW_FULLSCREEN,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F11),
        ),
        (
            PANE_SWAP,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::Tab),
        ),
        (
            PANE_EQUALIZE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::B),
        ),
        (
            VIEW_PREVIEW,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::P),
        ),
        (
            LOG_FOLLOW,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::F),
        ),
        (
            LOG_MARK,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::M),
        ),
        (
            PANE_ZOOM,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::X),
        ),
        (
            ZOOM_IN,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Plus),
        ),
        (
            ZOOM_IN,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Equals),
        ),
        (
            ZOOM_OUT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Minus),
        ),
        (
            ZOOM_RESET,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Num0),
        ),
        (
            PANE_ROTATE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::R),
        ),
        (
            PANE_RESIZE_LEFT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::ArrowLeft),
        ),
        (
            PANE_RESIZE_RIGHT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::ArrowRight),
        ),
        (
            PANE_RESIZE_UP,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::ArrowUp),
        ),
        (
            PANE_RESIZE_DOWN,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::ArrowDown),
        ),
        (
            PANE_SPLIT_H,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::E),
        ),
        (
            PANE_SPLIT_V,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::O),
        ),
        (
            TERM_COPY,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::C),
        ),
        (
            TERM_PASTE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::V),
        ),
        (
            NAV_UP,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::Backspace),
        ),
        (
            NAV_BACK,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::ArrowLeft),
        ),
        (
            NAV_FORWARD,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::ArrowRight),
        ),
        (
            NAV_REFRESH,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::R),
        ),
        (
            NAV_GOTO,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::L),
        ),
        (
            VIEW_HIDDEN,
            // Cmd+H hides the application on macOS; Finder's chord is Cmd+Shift+.
            if cfg!(target_os = "macos") {
                egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Period)
            } else {
                egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::H)
            },
        ),
        // macOS keyboards have no Delete key: Cmd+Backspace is the trash chord.
        (
            FILE_DELETE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Backspace),
        ),
        (
            FILE_COPY,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F5),
        ),
        (
            FILE_MOVE,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F6),
        ),
        (
            FILE_MKDIR,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F7),
        ),
        (
            FILE_NEWFILE,
            egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::F7),
        ),
        (
            FILE_RENAME,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F2),
        ),
        (
            FILE_DELETE,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F8),
        ),
        (
            FILE_DELETE,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::Delete),
        ),
        (
            FILE_DELETE_PERMANENT,
            egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::Delete),
        ),
        (
            EDIT_UNDO,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
        ),
        // Ctrl+A stays with the listing (and text fields), which select all themselves.
        (
            SELECT_NONE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::A),
        ),
        (
            SELECT_INVERT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::I),
        ),
        (
            SELECT_PATTERN,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
        ),
        (
            VIEW_FILTER,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::F),
        ),
        (
            BOOKMARK_ADD,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::D),
        ),
        (
            FILE_DUPLICATE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::D),
        ),
        (
            PROPERTIES,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::Enter),
        ),
        (
            FIND_FILES,
            egui::KeyboardShortcut::new(Modifiers::ALT, Key::F7),
        ),
        // Real windows deliver these as clipboard events instead; see `clipboard_event_ids`.
        (
            FILE_CLIP_COPY,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::C),
        ),
        (
            FILE_CLIP_CUT,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::X),
        ),
        (
            FILE_CLIP_PASTE,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::V),
        ),
    ]
}

/// Ctrl/Cmd+C, X and V reach egui as clipboard events, never as key presses. Take them out of
/// the frame's input and map them to the file-clipboard commands (paste carries the OS text).
fn clipboard_events(ctx: &egui::Context) -> Vec<(&'static str, Option<String>)> {
    ctx.input_mut(|input| {
        let mut found = Vec::new();
        input.events.retain(|event| match event {
            egui::Event::Copy => {
                found.push((FILE_CLIP_COPY, None));
                false
            }
            egui::Event::Cut => {
                found.push((FILE_CLIP_CUT, None));
                false
            }
            egui::Event::Paste(text) => {
                found.push((FILE_CLIP_PASTE, Some(text.clone())));
                false
            }
            _ => true,
        });
        found
    })
}

/// Run a command; ids that need input first open their dialog instead. A failure becomes the
/// visible notice and never crashes the frame.
pub(crate) fn dispatch(session: &mut Session, dialogs: &mut Dialogs, id: &str) {
    if let Some(dialog) = dialog_for(id, session) {
        dialogs.request(dialog);
        return;
    }
    if let Err(err) = session.dispatch(id, &Params::Null) {
        session.notice = Some(err.to_string());
    }
}

pub(crate) fn dispatch_folder(
    session: &mut Session,
    dialogs: &mut Dialogs,
    action: fm_ui::context_menu::FolderAction,
) {
    use fm_ui::dialogs::Dialog;
    let folder = action.folder;
    let dialog = match action.id {
        FILE_MKDIR => Some(Dialog::Mkdir {
            dir: folder.clone(),
            name: String::new(),
        }),
        FILE_NEWFILE => Some(Dialog::NewFile {
            dir: folder.clone(),
            name: String::new(),
        }),
        _ => None,
    };
    if let Some(dialog) = dialog {
        dialogs.request(dialog);
        return;
    }
    let folder = folder.to_string();
    let params = match action.id {
        "file.clip_paste" => serde_json::json!({ "dest": folder }),
        "nav.refresh" | "bookmark.add" | "preview.open" | "image.copy" | "media.play_folder" => {
            serde_json::json!({ "uri": folder })
        }
        _ => serde_json::json!({ "sources": [folder] }),
    };
    if let Err(err) = session.dispatch(action.id, &params) {
        session.notice = Some(err.to_string());
    }
}

/// Switch / open-saved / delete-saved submenus; the lists are data, so they are not constants.
fn workspace_lists(ui: &mut egui::Ui, session: &mut Session) {
    let mut run: Option<(&'static str, serde_json::Value)> = None;
    let tabs: Vec<String> = session
        .workspaces
        .tabs
        .iter()
        .map(|t| t.name.clone())
        .collect();
    let saved: Vec<String> = session
        .workspaces
        .saved
        .iter()
        .map(|w| w.name.clone())
        .collect();
    ui.separator();
    ui.add_enabled_ui(tabs.len() > 1, |ui| {
        ui.menu_button("Switch to", |ui| {
            for (ix, name) in tabs.iter().enumerate() {
                let active = ix == session.workspaces.active;
                if ui.selectable_label(active, name).clicked() {
                    run = Some(("workspace.switch", serde_json::json!({ "n": ix + 1 })));
                    ui.close();
                }
            }
        });
    });
    ui.add_enabled_ui(!saved.is_empty(), |ui| {
        ui.menu_button("Open saved", |ui| {
            for name in &saved {
                if ui.button(name).clicked() {
                    run = Some(("workspace.open", serde_json::json!({ "name": name })));
                    ui.close();
                }
            }
        });
        ui.menu_button("Delete saved", |ui| {
            for name in &saved {
                if ui.button(name).clicked() {
                    run = Some(("workspace.delete", serde_json::json!({ "name": name })));
                    ui.close();
                }
            }
        });
    });
    if let Some((id, params)) = run {
        session.dispatch_or_notice(id, &params);
    }
}

fn item(ui: &mut egui::Ui, session: &mut Session, dialogs: &mut Dialogs, id: &str) {
    let Some(spec) = session.registry.get(id) else {
        return;
    };
    let mut label = spec.label.to_string();
    // Remote locations have no trash: the same command deletes permanently there.
    if id == FILE_DELETE
        && session
            .layout
            .state(session.layout.focused())
            .is_some_and(|p| p.location.scheme() != fm_vfs::Scheme::File)
    {
        label = "Delete Permanently".to_string();
    }
    let enabled = (spec.enabled)(session);
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = spec.shortcut {
        // Right-aligned, weak-styled after the label (egui menu convention).
        button = button.shortcut_text(shortcut);
    }
    if ui.add_enabled(enabled, button).clicked() {
        if id == TERM_PASTE {
            fm_ui::terminal::request_paste(ui.ctx());
        } else {
            dispatch(session, dialogs, id);
        }
        ui.close();
    }
}

/// A menu entry rendered as a checkbox; clicking dispatches the toggle command.
fn check_item(ui: &mut egui::Ui, session: &mut Session, dialogs: &mut Dialogs, id: &str, on: bool) {
    let Some(spec) = session.registry.get(id) else {
        return;
    };
    let label = spec.label.to_string();
    let shortcut = spec.shortcut;
    let mut checked = on;
    let mut text = egui::RichText::new(label);
    if let Some(sc) = shortcut {
        text = egui::RichText::new(format!("{}    {sc}", text.text()));
    }
    if ui.checkbox(&mut checked, text).clicked() {
        dispatch(session, dialogs, id);
        ui.close();
    }
}

fn theme_radio(
    ui: &mut egui::Ui,
    session: &mut Session,
    dialogs: &mut Dialogs,
    id: &str,
    expected: ThemePreference,
) {
    let selected = session.prefs.theme == expected;
    let label = match expected {
        ThemePreference::System => "System",
        ThemePreference::Dark => "Dark",
        ThemePreference::Light => "Light",
    };
    if ui.selectable_label(selected, label).clicked() {
        dispatch(session, dialogs, id);
    }
}

/// Go → Places: the standard folders and bookmarks, opened in the focused pane.
fn places_menu(ui: &mut egui::Ui, session: &mut Session) {
    let places = session.places();
    ui.menu_button("Places", |ui| {
        let mut bookmarks = false;
        for place in places {
            if place.bookmark && !bookmarks {
                bookmarks = true;
                ui.separator();
            }
            if ui
                .button(&place.name)
                .on_hover_text(place.uri.to_string())
                .clicked()
            {
                session.dispatch_or_notice(
                    "nav.open",
                    &serde_json::json!({ "uri": place.uri.to_string() }),
                );
                ui.close();
            }
        }
    });
}

fn devices_menu(ui: &mut egui::Ui, session: &mut Session) {
    let devices = session.volumes.entries().to_vec();
    if devices.is_empty() {
        return;
    }
    ui.menu_button("Devices", |ui| {
        for device in &devices {
            ui.menu_button(fm_engine::devices::display_label(&device.name), |ui| {
                ui.label(fm_engine::devices::display_label(
                    &fm_engine::item_cmds::display_path(&device.mount),
                ));
                if ui.button("Open").clicked() {
                    session.dispatch_or_notice(
                        "nav.open",
                        &serde_json::json!({"uri": device.mount.to_string()}),
                    );
                    ui.close();
                }
                if fm_ui::places::device_removal(ui, session, device) {
                    ui.close();
                }
            });
        }
        ui.separator();
        if ui
            .add_enabled(
                !session.volumes.scanning() && !session.volumes.removing(),
                egui::Button::new("Refresh Devices"),
            )
            .clicked()
        {
            session.dispatch_or_notice("devices.refresh", &serde_json::Value::Null);
            ui.close();
        }
    });
}

/// The top menu bar (File / Panes / View / Help).
pub fn menu_bar(ui: &mut egui::Ui, session: &mut Session, dialogs: &mut Dialogs) {
    egui::menu::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, session, dialogs, FILE_COPY);
            item(ui, session, dialogs, FILE_MOVE);
            item(ui, session, dialogs, FILE_MKDIR);
            item(ui, session, dialogs, FILE_NEWFILE);
            item(ui, session, dialogs, FILE_RENAME);
            item(ui, session, dialogs, FILE_DUPLICATE);
            ui.separator();
            item(ui, session, dialogs, OPEN_WITH);
            item(ui, session, dialogs, COMPARE_WITH);
            item(ui, session, dialogs, PROPERTIES);
            item(ui, session, dialogs, FIND_FILES);
            ui.separator();
            item(ui, session, dialogs, REMOTE_CONNECT);
            item(ui, session, dialogs, NETWORK_CONNECT);
            item(ui, session, dialogs, NETWORK_OPEN);
            item(ui, session, dialogs, REMOTE_RECONNECT);
            item(ui, session, dialogs, REMOTE_DISCONNECT);
            ui.separator();
            item(ui, session, dialogs, FILE_DELETE);
            item(ui, session, dialogs, FILE_DELETE_PERMANENT);
            item(ui, session, dialogs, TRASH_OPEN);
            ui.separator();
            item(ui, session, dialogs, PREFERENCES);
            ui.separator();
            item(ui, session, dialogs, QUIT);
        });
        ui.menu_button("Edit", |ui| {
            item(ui, session, dialogs, EDIT_UNDO);
            ui.separator();
            item(ui, session, dialogs, FILE_CLIP_CUT);
            item(ui, session, dialogs, FILE_CLIP_COPY);
            item(ui, session, dialogs, FILE_CLIP_PASTE);
            ui.separator();
            item(ui, session, dialogs, COPY_PATH);
            item(ui, session, dialogs, COPY_NAME);
            item(ui, session, dialogs, IMAGE_COPY);
            ui.separator();
            item(ui, session, dialogs, SELECT_ALL);
            item(ui, session, dialogs, SELECT_NONE);
            item(ui, session, dialogs, SELECT_INVERT);
            item(ui, session, dialogs, SELECT_PATTERN);
        });
        ui.menu_button("Go", |ui| {
            item(ui, session, dialogs, NAV_GOTO);
            ui.separator();
            item(ui, session, dialogs, NAV_UP);
            item(ui, session, dialogs, NAV_BACK);
            item(ui, session, dialogs, NAV_FORWARD);
            item(ui, session, dialogs, NAV_REFRESH);
            ui.separator();
            item(ui, session, dialogs, BOOKMARK_ADD);
            places_menu(ui, session);
            devices_menu(ui, session);
        });
        ui.menu_button("Workspace", |ui| {
            item(ui, session, dialogs, WS_NEW);
            item(ui, session, dialogs, WS_RENAME);
            item(ui, session, dialogs, WS_SAVE);
            item(ui, session, dialogs, WS_CLOSE);
            workspace_lists(ui, session);
        });
        ui.menu_button("Panes", |ui| {
            item(ui, session, dialogs, PANE_SPLIT_H);
            item(ui, session, dialogs, PANE_SPLIT_V);
            ui.separator();
            item(ui, session, dialogs, PANE_SWAP);
            item(ui, session, dialogs, PANE_FOCUS_NEXT);
            ui.separator();
            item(ui, session, dialogs, PANE_ZOOM);
            item(ui, session, dialogs, PANE_EQUALIZE);
            item(ui, session, dialogs, PANE_ROTATE);
            ui.menu_button("Resize", |ui| {
                item(ui, session, dialogs, PANE_RESIZE_LEFT);
                item(ui, session, dialogs, PANE_RESIZE_RIGHT);
                item(ui, session, dialogs, PANE_RESIZE_UP);
                item(ui, session, dialogs, PANE_RESIZE_DOWN);
            });
            ui.separator();
            item(ui, session, dialogs, PANE_CLOSE);
        });
        ui.menu_button("Terminal", |ui| {
            item(ui, session, dialogs, TERM_COPY);
            item(ui, session, dialogs, TERM_PASTE);
            ui.separator();
            item(ui, session, dialogs, TERM_CD_HERE);
            item(ui, session, dialogs, TERM_CLEAR);
            item(ui, session, dialogs, TERM_RESTART);
        });
        ui.menu_button("View", |ui| {
            // Unlike Ctrl+H and the status bar (this session only), the menu item also saves
            // the choice as the startup default.
            let mut shown = session.show_hidden;
            if ui
                .checkbox(&mut shown, "Show Hidden Files (remember)")
                .on_hover_text("Ctrl+H toggles for this session only")
                .clicked()
            {
                session.dispatch_or_notice(
                    "prefs.set",
                    &serde_json::json!({"key": "show_hidden", "value": shown}),
                );
                ui.close();
            }
            check_item(ui, session, dialogs, VIEW_LINKS, session.show_link_targets);
            ui.separator();
            item(ui, session, dialogs, VIEW_FILTER);
            item(ui, session, dialogs, VIEW_FILTER_CLEAR);
            ui.separator();
            item(ui, session, dialogs, ZOOM_IN);
            item(ui, session, dialogs, ZOOM_OUT);
            item(ui, session, dialogs, ZOOM_RESET);
            ui.separator();
            item(ui, session, dialogs, VIEW_PREVIEW);
            ui.menu_button("Diff", |ui| {
                for id in [
                    "preview.diff.toggle",
                    DIFF_NEXT,
                    DIFF_PREV,
                    "preview.diff.mode",
                    "preview.diff.eol",
                    "preview.diff.reload",
                ] {
                    item(ui, session, dialogs, id);
                }
                item(ui, session, dialogs, "git.refresh");
            });
            ui.menu_button("Preview", |ui| {
                item(ui, session, dialogs, PREVIEW_SELECT_ALL);
                item(ui, session, dialogs, PREVIEW_COPY_TEXT);
                item(ui, session, dialogs, PREVIEW_SCALE_IN);
                item(ui, session, dialogs, PREVIEW_SCALE_OUT);
                item(ui, session, dialogs, PREVIEW_SCALE_RESET);
                item(ui, session, dialogs, PREVIEW_PRETTY);
                item(ui, session, dialogs, PREVIEW_SOURCE);
                item(ui, session, dialogs, PREVIEW_WRAP);
                item(ui, session, dialogs, PREVIEW_FIND);
                ui.separator();
                item(ui, session, dialogs, THUMBS_TOGGLE);
                item(ui, session, dialogs, THUMBS_SIZE);
                item(ui, session, dialogs, THUMBS_CLEAR);
                item(ui, session, dialogs, VIDEO_PLAY);
                item(ui, session, dialogs, VIDEO_MUTE);
                item(ui, session, dialogs, AUDIO_PLAY);
                item(ui, session, dialogs, AUDIO_MUTE);
                ui.separator();
                item(ui, session, dialogs, LOG_FOLLOW);
                item(ui, session, dialogs, LOG_COLUMNS);
                item(ui, session, dialogs, LOG_MARK);
                item(ui, session, dialogs, LOG_EARLIER);
            });
            ui.separator();
            ui.menu_button("Theme", |ui| {
                theme_radio(ui, session, dialogs, THEME_SYSTEM, ThemePreference::System);
                theme_radio(ui, session, dialogs, THEME_DARK, ThemePreference::Dark);
                theme_radio(ui, session, dialogs, THEME_LIGHT, ThemePreference::Light);
                ui.separator();
                item(ui, session, dialogs, THEME_TOGGLE);
            });
            ui.menu_button("Sort", |ui| {
                item(ui, session, dialogs, SORT_NAME);
                item(ui, session, dialogs, SORT_SIZE);
                item(ui, session, dialogs, SORT_MTIME);
                item(ui, session, dialogs, SORT_TYPE);
                ui.separator();
                item(ui, session, dialogs, SORT_DIRS_FIRST);
            });
            ui.separator();
            item(ui, session, dialogs, WINDOW_FULLSCREEN);
        });
        ui.menu_button("Help", |ui| {
            item(ui, session, dialogs, CHECK_UPDATES);
            item(ui, session, dialogs, ABOUT);
        });
    });
}

fn modifier_count(m: &egui::Modifiers) -> u8 {
    u8::from(m.alt) + u8::from(m.shift) + u8::from(m.command) + u8::from(m.ctrl)
}

/// Apply keyboard shortcuts; returns the ids that fired.
///
/// Call this at the top of the frame, before any widget renders: `Tab` is also
fn terminal_shortcut(shortcut: &egui::KeyboardShortcut) -> bool {
    let m = shortcut.modifiers;
    m.mac_cmd || (m.shift && m.command)
}

/// egui's focus-traversal key, and its `focus_direction` is committed at
/// `begin_pass`. Consuming the shortcut here (and cancelling the direction for
/// `pane.swap`) keeps `Tab` swapping panes instead of tabbing through widgets.
pub fn consume_shortcuts(
    ctx: &egui::Context,
    session: &mut Session,
    dialogs: &mut Dialogs,
) -> Vec<&'static str> {
    let mut fired = Vec::new();
    // File and bare-key shortcuts (Backspace, Tab) must not fire while a dialog or text field owns the keyboard.
    // The SFTP connect popup is a form too: Tab walks its fields instead of swapping panes.
    let on_form = fm_ui::dialogs::window_owns_keyboard(session);
    let typing = dialogs.is_open() || ctx.text_edit_focused() || on_form;
    // Most modifiers first, so Cmd+Backspace wins over plain Backspace.
    let mut table = shortcuts();
    table.sort_by_key(|(_, sc)| std::cmp::Reverse(modifier_count(&sc.modifiers)));
    // A focused terminal owns the keyboard: Ctrl+<key>, Backspace, Tab, F-keys and friends
    // belong to the shell. Only Cmd/Ctrl+Shift chords (and macOS Cmd) stay app shortcuts.
    let terminal = !typing && session.focused_kind() == Some(PaneKind::Terminal);
    let files_focused = session.focused_kind().is_none_or(|k| k.is_files());
    for (id, shortcut) in table {
        if terminal && !terminal_shortcut(&shortcut) {
            continue;
        }
        // Clipboard shortcuts arrive as `Event::Copy` / `Event::Paste`; the terminal widget
        // needs those events (and their text/modifiers) to distinguish copy from shell input.
        if terminal && (id == TERM_COPY || id == TERM_PASTE) {
            continue;
        }
        if typing && (id.starts_with("term.") || id.starts_with("pane.resize.")) {
            continue;
        }
        if !files_focused && id.starts_with("file.clip_") {
            continue;
        }
        if id == VIEW_FILTER
            && session.focused_kind() == Some(PaneKind::Preview)
            && (dialogs.is_open() || on_form)
        {
            continue;
        }
        if typing
            && (id.starts_with("file.")
                || id.starts_with("edit.")
                || id == PROPERTIES
                || shortcut.modifiers.is_none())
        {
            continue;
        }
        let consumed = ctx.input_mut(|input| input.consume_shortcut(&shortcut));
        if consumed {
            let id = if id == VIEW_FILTER
                && session.focused_kind() == Some(PaneKind::Preview)
                && !dialogs.is_open()
                && !on_form
            {
                PREVIEW_FIND
            } else {
                id
            };
            let id = if !typing && session.focused_kind() == Some(PaneKind::Preview) {
                match id {
                    ZOOM_IN => PREVIEW_SCALE_IN,
                    ZOOM_OUT => PREVIEW_SCALE_OUT,
                    ZOOM_RESET => PREVIEW_SCALE_RESET,
                    // A diff borrows the F7 pair for change navigation (as in IDE diff viewers).
                    FILE_MKDIR if session.command_enabled(DIFF_NEXT) => DIFF_NEXT,
                    FILE_NEWFILE if session.command_enabled(DIFF_PREV) => DIFF_PREV,
                    _ => id,
                }
            } else {
                id
            };
            if id == FILE_DELETE && !typing && session.focused_kind() == Some(PaneKind::Media) {
                // The playlist owns the selection; it removes the selected rows next frame.
                ctx.data_mut(|d| d.insert_temp(fm_ui::media::delete_request_id(), true));
                fired.push(id);
                continue;
            }
            dispatch(session, dialogs, id);
            fired.push(id);
            if id == PANE_SWAP {
                // Cancel the focus move egui already staged for this `Tab`.
                ctx.memory_mut(|mem| mem.move_focus(egui::FocusDirection::None));
            }
        }
    }
    // Preview, trash and other non-file panes leave Ctrl+C to egui so selected text is copied.
    if !terminal && !typing && files_focused {
        for (id, os_text) in clipboard_events(ctx) {
            match os_text {
                Some(text) => {
                    session.dispatch_or_notice(id, &serde_json::json!({ "os_text": text }))
                }
                None => session.dispatch_or_notice(id, &serde_json::Value::Null),
            };
            fired.push(id);
        }
    }
    if !terminal && !dialogs.is_open() {
        for id in fm_ui::media_keys::handle(ctx, session, typing) {
            fired.push(id);
        }
    }
    if !terminal && !typing && !dialogs.is_open() {
        use egui::Key::*;
        let digits = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
        for (ix, key) in digits.into_iter().enumerate() {
            let sc = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key);
            if ctx.input_mut(|input| input.consume_shortcut(&sc)) {
                if key == Num0 && session.focused_kind() == Some(PaneKind::Preview) {
                    dispatch(session, dialogs, PREVIEW_SCALE_RESET);
                    fired.push(PREVIEW_SCALE_RESET);
                } else {
                    session.dispatch_or_notice(
                        "workspace.switch",
                        &serde_json::json!({ "n": ix + 1 }),
                    );
                    fired.push("workspace.switch");
                }
            }
        }
    }
    if terminal {
        let swap = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Tab,
        );
        if ctx.input_mut(|input| input.consume_shortcut(&swap)) {
            dispatch(session, dialogs, PANE_SWAP);
            fired.push(PANE_SWAP);
        }
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_context_action_dispatches_the_uri_and_does_not_open_a_dialog() {
        use std::sync::Arc;
        let mut s = Session::new().expect("session");
        let mem = fm_testkit::MemFs::new();
        mem.put_str("/d/test.txt", b"preview").expect("file");
        let file = fm_vfs::VfsUri::parse("/d/test.txt").expect("uri");
        let dir = file.parent().expect("parent");
        s.backends.register(Arc::new(mem));
        s.layout.state_mut(1).expect("pane").location = dir.clone();
        s.submit_read(dir).expect("read");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !s.pane_view_ready(1) {
            s.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut dialogs = Dialogs::default();
        dispatch_folder(
            &mut s,
            &mut dialogs,
            fm_ui::context_menu::FolderAction {
                id: "preview.open",
                folder: file,
            },
        );
        assert!(dialogs.open.is_none());
        assert!(s.notice.is_none(), "{:?}", s.notice);
        assert_eq!(s.layout.panes().len(), 2);
    }

    #[test]
    fn folder_context_commands_keep_the_explicit_folder_target() {
        use fm_ui::{context_menu::FolderAction, dialogs::Dialog};
        let mut s = Session::new().expect("session");
        let folder = fm_vfs::VfsUri::parse("/clicked/subfolder").expect("uri");
        let mut dialogs = Dialogs::default();
        dispatch_folder(
            &mut s,
            &mut dialogs,
            FolderAction {
                id: FILE_MKDIR,
                folder: folder.clone(),
            },
        );
        assert_eq!(
            dialogs.open,
            Some(Dialog::Mkdir {
                dir: folder.clone(),
                name: String::new()
            })
        );
        dispatch_folder(
            &mut s,
            &mut dialogs,
            FolderAction {
                id: FILE_NEWFILE,
                folder: folder.clone(),
            },
        );
        assert_eq!(
            dialogs.open,
            Some(Dialog::NewFile {
                dir: folder.clone(),
                name: String::new()
            })
        );
        dispatch_folder(
            &mut s,
            &mut dialogs,
            FolderAction {
                id: "file.copy_path",
                folder: folder.clone(),
            },
        );
        assert_eq!(s.clipboard_out.as_deref(), Some("/clicked/subfolder"));
        dispatch_folder(
            &mut s,
            &mut dialogs,
            FolderAction {
                id: "file.copy_name",
                folder,
            },
        );
        assert_eq!(s.clipboard_out.as_deref(), Some("subfolder"));
    }

    #[test]
    fn media_folder_context_dispatch_preserves_the_explicit_target() {
        let mut s = Session::new().expect("session");
        let fs = fm_testkit::MemFs::new();
        fs.mkdirs_str("/clicked/folder").expect("folder");
        s.backends.register(std::sync::Arc::new(fs));
        dispatch_folder(
            &mut s,
            &mut Dialogs::default(),
            fm_ui::context_menu::FolderAction {
                id: "media.play_folder",
                folder: fm_vfs::VfsUri::parse("/clicked/folder").expect("uri"),
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !s.command_enabled("media.drop") {
            s.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(s.messages.recent().any(|message| {
            message
                .text
                .contains("No supported local media files in folder")
        }));
        assert!(s.media.queue.items().is_empty());
    }

    #[test]
    fn clipboard_events_become_file_clipboard_commands() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            events: vec![
                egui::Event::Copy,
                egui::Event::Cut,
                egui::Event::Paste("/a".into()),
            ],
            ..Default::default()
        };
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx();
            let found = clipboard_events(ctx);
            assert_eq!(
                found,
                vec![
                    (FILE_CLIP_COPY, None),
                    (FILE_CLIP_CUT, None),
                    (FILE_CLIP_PASTE, Some("/a".to_string())),
                ]
            );
            assert!(clipboard_events(ctx).is_empty(), "events are taken once");
        });
        out.textures_delta.clear();
    }

    #[test]
    fn every_menu_and_shortcut_id_is_registered() {
        let session = Session::new().expect("session");
        for id in all_ids() {
            assert!(session.registry.contains(id), "dead menu id: {id}");
        }
        for (id, _) in shortcuts() {
            assert!(session.registry.contains(id), "dead shortcut id: {id}");
        }
    }

    #[test]
    fn shortcut_display_strings_agree_with_the_registry() {
        let session = Session::new().expect("session");
        for (id, shortcut) in shortcuts() {
            let spec = session.registry.get(id).expect("registered");
            let display = spec.shortcut.unwrap_or("(none)");
            let expected = format_shortcut(&shortcut);
            let is_alias = (id == FILE_DELETE
                && (expected == "Delete" || expected == "Ctrl+Backspace"))
                || (id == ZOOM_IN && expected == "Ctrl+Shift+=");
            assert!(
                is_alias || display == expected,
                "shortcut display mismatch for {id}"
            );
        }
    }

    /// Formatter for the bindings we use.
    fn format_shortcut(shortcut: &egui::KeyboardShortcut) -> String {
        let mut parts = Vec::new();
        if shortcut.modifiers.command {
            parts.push("Ctrl");
        }
        if shortcut.modifiers.shift {
            parts.push("Shift");
        }
        if shortcut.modifiers.alt {
            parts.push("Alt");
        }
        if shortcut.modifiers.mac_cmd {
            parts.push("Cmd");
        }
        parts.push(key_name(shortcut.logical_key));
        parts.join("+")
    }

    fn key_name(key: egui::Key) -> &'static str {
        use egui::Key;
        match key {
            Key::Q => "Q",
            Key::C => "C",
            Key::V => "V",
            Key::Comma => ",",
            Key::T => "T",
            Key::H => "H",
            Key::R => "R",
            Key::L => "L",
            Key::Z => "Z",
            Key::F2 => "F2",
            Key::F11 => "F11",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F7 => "F7",
            Key::F8 => "F8",
            Key::Delete => "Delete",
            Key::Period => ".",
            Key::Tab => "Tab",
            Key::Backspace => "Backspace",
            Key::ArrowLeft => "Left",
            Key::ArrowRight => "Right",
            Key::ArrowUp => "Up",
            Key::ArrowDown => "Down",
            Key::E => "E",
            Key::B => "B",
            Key::P => "P",
            Key::F => "F",
            Key::M => "M",
            Key::O => "O",
            Key::X => "X",
            Key::W => "W",
            Key::Plus => "+",
            Key::Equals => "=",
            Key::Minus => "-",
            Key::Num0 => "0",
            Key::A => "A",
            Key::I => "I",
            Key::S => "S",
            Key::D => "D",
            Key::Enter => "Enter",
            _ => "?",
        }
    }

    fn press(ctx: &egui::Context, session: &mut Session, dialogs: &mut Dialogs, key: egui::Key) {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let mut out = ctx.run_ui(input, |ui| {
            consume_shortcuts(ui.ctx(), session, dialogs);
        });
        out.textures_delta.clear();
    }

    fn chord(
        ctx: &egui::Context,
        session: &mut Session,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> Vec<&'static str> {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        });
        let mut fired = Vec::new();
        let mut dialogs = Dialogs::default();
        let mut out = ctx.run_ui(input, |ui| {
            fired = consume_shortcuts(ui.ctx(), session, &mut dialogs);
        });
        out.textures_delta.clear();
        fired
    }

    #[test]
    fn preview_find_chord_is_routed_away_from_files_even_while_typing() {
        let mut s = Session::new().expect("session");
        s.dispatch("view.preview.toggle", &serde_json::Value::Null)
            .expect("preview");
        let pane = s
            .layout
            .panes()
            .into_iter()
            .find(|p| s.pane_kind(*p) == Some(PaneKind::Preview))
            .expect("preview pane");
        s.layout.focus_pane(pane).expect("focus");
        let ctx = egui::Context::default();
        // Loading/empty previews reject find gracefully, but must never trigger the Files filter.
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::F, egui::Modifiers::COMMAND),
            [PREVIEW_FIND]
        );
        let mut value = String::new();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut value))
                .request_focus();
        });
        output.textures_delta.clear();
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::F, egui::Modifiers::COMMAND),
            [PREVIEW_FIND]
        );
        assert!(s.layout.state(1).expect("files").listing.filter.is_none());
        s.preferences_open = true;
        assert!(chord(&ctx, &mut s, egui::Key::F, egui::Modifiers::COMMAND).is_empty());
        s.preferences_open = false;
        s.layout.focus_pane(1).expect("focus files");
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::F, egui::Modifiers::COMMAND),
            [VIEW_FILTER]
        );
    }

    #[test]
    fn focused_preview_zoom_chords_never_change_global_zoom() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        let global = s.prefs.zoom;
        let ctx = egui::Context::default();
        for (key, id) in [
            (egui::Key::Plus, PREVIEW_SCALE_IN),
            (egui::Key::Minus, PREVIEW_SCALE_OUT),
            (egui::Key::Num0, PREVIEW_SCALE_RESET),
        ] {
            assert_eq!(
                chord(
                    &ctx,
                    &mut s,
                    key,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
                ),
                [id]
            );
            assert_eq!(s.prefs.zoom, global);
        }
        assert_eq!(s.preview_slot(pane).expect("slot").scale, 1.0);
    }

    #[test]
    fn ctrl_zero_resets_preview_instead_of_switching_to_workspace_ten() {
        let mut s = Session::new().expect("session");
        for _ in 1..10 {
            s.dispatch("workspace.new", &Params::Null).expect("new tab");
        }
        s.dispatch("workspace.switch", &serde_json::json!({"n": 1}))
            .expect("first tab");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        s.dispatch(PREVIEW_SCALE_IN, &Params::Null).expect("zoom");
        let global = s.prefs.zoom;
        let ctx = egui::Context::default();
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::Num0, egui::Modifiers::COMMAND),
            [PREVIEW_SCALE_RESET]
        );
        assert_eq!(s.workspaces.active, 0);
        assert_eq!(s.preview_slot(pane).expect("preview").scale, 1.0);
        assert_eq!(s.prefs.zoom, global);
        assert!(s.notice.is_none());
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::Num2, egui::Modifiers::COMMAND),
            ["workspace.switch"]
        );
        assert_eq!(
            s.workspaces.active, 1,
            "other digits still switch from Preview"
        );
        s.dispatch("workspace.switch", &serde_json::json!({"n": 1}))
            .expect("return to first tab");
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Files;
        assert_eq!(
            chord(&ctx, &mut s, egui::Key::Num0, egui::Modifiers::COMMAND),
            ["workspace.switch"]
        );
        assert_eq!(s.workspaces.active, 9);
    }

    #[test]
    fn ctrl_zero_stays_with_preview_text_fields_dialogs_and_terminal_input() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Preview;
        s.dispatch(PREVIEW_SCALE_IN, &Params::Null).expect("zoom");
        let scale = s.preview_slot(pane).expect("preview").scale;
        s.preferences_open = true;
        let ctx = egui::Context::default();
        assert!(chord(&ctx, &mut s, egui::Key::Num0, egui::Modifiers::COMMAND).is_empty());
        s.preferences_open = false;
        let mut value = String::new();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            ui.add(egui::TextEdit::singleline(&mut value))
                .request_focus();
        });
        out.textures_delta.clear();
        assert!(chord(&ctx, &mut s, egui::Key::Num0, egui::Modifiers::COMMAND).is_empty());
        assert_eq!(s.preview_slot(pane).expect("preview").scale, scale);
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Terminal;
        let ctx = egui::Context::default();
        assert!(chord(&ctx, &mut s, egui::Key::Num0, egui::Modifiers::COMMAND).is_empty());
        assert_eq!(s.workspaces.active, 0);
    }

    #[test]
    fn preferences_over_a_terminal_leave_clipboard_events_for_the_form() {
        let mut session = Session::new().expect("session");
        let pane = session.layout.focused();
        session.layout.state_mut(pane).expect("pane").kind = PaneKind::Terminal;
        session.preferences_open = true;
        let ctx = egui::Context::default();
        let mut dialogs = Dialogs::default();
        let events = vec![
            egui::Event::Copy,
            egui::Event::Cut,
            egui::Event::Paste("#68b7dc".into()),
        ];
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: events.clone(),
                ..Default::default()
            },
            |ui| {
                assert!(consume_shortcuts(ui.ctx(), &mut session, &mut dialogs).is_empty());
                assert_eq!(ui.input(|i| i.events.clone()), events);
            },
        );
        out.textures_delta.clear();
        assert!(session.notice.is_none());
    }

    #[test]
    fn a_focused_terminal_keeps_plain_and_ctrl_keys_for_the_shell() {
        use egui::{Key, Modifiers};
        let mut session = Session::new().expect("session");
        let pane = session.layout.focused();
        session.layout.state_mut(pane).expect("pane").kind = PaneKind::Terminal;
        let ctx = egui::Context::default();
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let ctrl_shift = Modifiers {
            shift: true,
            ..ctrl
        };
        for (key, m) in [
            (Key::Backspace, Modifiers::NONE),
            (Key::Tab, Modifiers::NONE),
            (Key::F5, Modifiers::NONE),
            (Key::L, ctrl),
            (Key::F, ctrl),
            (Key::C, ctrl),
            (Key::D, ctrl),
            (Key::R, ctrl),
        ] {
            assert!(
                chord(&ctx, &mut session, key, m).is_empty(),
                "{key:?} {m:?}"
            );
        }
        let copy = chord(&ctx, &mut session, Key::C, ctrl_shift);
        assert!(copy.is_empty());
        let paste = chord(&ctx, &mut session, Key::V, ctrl_shift);
        assert!(paste.is_empty());
        let swap = chord(&ctx, &mut session, Key::Tab, ctrl_shift);
        assert_eq!(swap, vec![PANE_SWAP]);
    }

    #[test]
    fn arrangement_chords_work_with_a_terminal_focused() {
        use egui::{Key, Modifiers};
        let ctx = egui::Context::default();
        let mut session = Session::new().expect("session");
        session
            .dispatch("pane.split.horizontal", &serde_json::Value::Null)
            .expect("split");
        session
            .dispatch("pane.kind.set", &serde_json::json!({"kind": "terminal"}))
            .expect("kind");
        let cs = Modifiers::COMMAND | Modifiers::SHIFT;
        for (key, id) in [
            (Key::B, PANE_EQUALIZE),
            (Key::E, PANE_SPLIT_H),
            (Key::O, PANE_SPLIT_V),
            (Key::X, PANE_ZOOM),
            (Key::R, PANE_ROTATE),
            (Key::ArrowLeft, PANE_RESIZE_LEFT),
            (Key::ArrowDown, PANE_RESIZE_DOWN),
        ] {
            assert_eq!(chord(&ctx, &mut session, key, cs), vec![id], "{id}");
        }
    }

    #[test]
    fn files_panes_keep_their_shortcuts() {
        use egui::{Key, Modifiers};
        let mut session = Session::new().expect("session");
        let ctx = egui::Context::default();
        assert_eq!(
            chord(&ctx, &mut session, Key::Backspace, Modifiers::NONE),
            vec![NAV_UP]
        );
    }

    #[test]
    fn f7_opens_dialog_with_a_directory_selected() {
        let mem = fm_testkit::MemFs::new();
        mem.mkdirs_str("/h/docs").expect("mkdirs");
        let mut session = Session::default();
        session.backends.register(std::sync::Arc::new(mem));
        let uri = fm_vfs::VfsUri::parse("/h").expect("uri");
        session.layout.state_mut(1).expect("pane").location = uri;
        session
            .layout
            .state_mut(1)
            .expect("pane")
            .listing
            .select_only(Some("docs"), 0);
        let ctx = egui::Context::default();
        let mut dialogs = Dialogs::default();
        press(&ctx, &mut session, &mut dialogs, egui::Key::F7);
        assert!(dialogs.is_open());
    }

    #[test]
    fn dispatching_shortcut_targets_changes_theme() {
        let mut session = Session::new().expect("session");
        session.dispatch(THEME_DARK, &Params::Null).expect("dark");
        assert_eq!(session.prefs.theme, ThemePreference::Dark);
        dispatch(&mut session, &mut Dialogs::default(), THEME_TOGGLE);
        assert_eq!(session.prefs.theme, ThemePreference::Light);
        dispatch(&mut session, &mut Dialogs::default(), ABOUT);
        assert!(session.about_open);
        dispatch(&mut session, &mut Dialogs::default(), QUIT);
        assert!(!session.running);
    }

    #[test]
    fn pane_commands_dispatch_through_the_session() {
        let mut session = Session::new().expect("session");
        dispatch(&mut session, &mut Dialogs::default(), PANE_SPLIT_H);
        assert_eq!(session.layout.panes().len(), 2);
        dispatch(&mut session, &mut Dialogs::default(), PANE_FOCUS_NEXT);
        dispatch(&mut session, &mut Dialogs::default(), PANE_SWAP);
        dispatch(&mut session, &mut Dialogs::default(), PANE_CLOSE);
        assert_eq!(session.layout.panes().len(), 1);
    }

    fn collect_shapes(shapes: &[egui::epaint::ClippedShape], out: &mut Vec<String>) {
        fn walk(shapes: &[egui::Shape], out: &mut Vec<String>) {
            for shape in shapes {
                match shape {
                    egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                    egui::Shape::Vec(inner) => walk(inner, out),
                    _ => {}
                }
            }
        }
        for clipped in shapes {
            walk(std::slice::from_ref(&clipped.shape), out);
        }
    }

    /// A command with a keyboard binding paints that binding after its label
    /// in the menu row (`Button::shortcut_text` in `item`).
    #[test]
    fn menu_items_show_their_shortcut_after_the_label() {
        let mut session = Session::new().expect("session");
        let ctx = egui::Context::default();
        for (id, _) in shortcuts() {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE)
                        .show(ctx, |ui| {
                            egui::Frame::NONE.show(ui, |ui| {
                                item(ui, &mut session, &mut Dialogs::default(), id);
                            });
                        });
                },
            );
            output.textures_delta.clear();
            let mut texts = Vec::new();
            collect_shapes(&output.shapes, &mut texts);
            let painted = texts.join(" ");
            let spec = session.registry.get(id).expect("registered");
            let label = spec.label;
            let shortcut = spec
                .shortcut
                .unwrap_or_else(|| panic!("{id}: binding without a display string"));
            let label_at = painted
                .find(label)
                .unwrap_or_else(|| panic!("{id}: label {label:?} not painted: {painted:?}"));
            let shortcut_at = painted
                .find(shortcut)
                .unwrap_or_else(|| panic!("{id}: shortcut {shortcut:?} not painted: {painted:?}"));
            assert!(
                shortcut_at >= label_at + label.len(),
                "{id}: shortcut {shortcut:?} must be painted after {label:?} \
                 (painted: {painted:?})"
            );
        }
    }
}
