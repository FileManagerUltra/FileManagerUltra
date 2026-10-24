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
pub const THEME_DARK: &str = "view.theme.dark";
pub const THEME_LIGHT: &str = "view.theme.light";
pub const THEME_SYSTEM: &str = "view.theme.system";
pub const THEME_TOGGLE: &str = "view.theme.toggle";
pub const PANE_SPLIT_H: &str = "pane.split.horizontal";
pub const PANE_SPLIT_V: &str = "pane.split.vertical";
pub const PANE_CLOSE: &str = "pane.close";
pub const PANE_SWAP: &str = "pane.swap";
pub const PANE_EQUALIZE: &str = "pane.equalize";
pub const PANE_ZOOM: &str = "pane.zoom";
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
pub const PREVIEW_WRAP: &str = "preview.toggle_wrap";
pub const LOG_FOLLOW: &str = "preview.log.follow";
pub const LOG_COLUMNS: &str = "preview.log.columns";
pub const LOG_MARK: &str = "preview.log.mark";
pub const THUMBS_TOGGLE: &str = "preview.thumbs.toggle";
pub const THUMBS_SIZE: &str = "preview.thumbs.size";
pub const THUMBS_CLEAR: &str = "preview.thumbs.clear_cache";
pub const VIDEO_PLAY: &str = "preview.video.play";
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
pub const FILE_DELETE: &str = "file.delete";
pub const FILE_DELETE_PERMANENT: &str = "file.delete_permanent";
pub const EDIT_UNDO: &str = "edit.undo";
pub const FILE_CLIP_COPY: &str = "file.clip_copy";
pub const FILE_CLIP_CUT: &str = "file.clip_cut";
pub const FILE_CLIP_PASTE: &str = "file.clip_paste";
pub const WS_NEW: &str = "workspace.new";
pub const WS_CLOSE: &str = "workspace.close";
pub const WS_SAVE: &str = "workspace.save";
pub const WS_RENAME: &str = "workspace.rename";

/// Every command id referenced by the menus or the shortcut table.
/// (Used by the tests and by `cargo xtask brand-check`'s registry cross-check.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn all_ids() -> &'static [&'static str] {
    &[
        QUIT,
        ABOUT,
        PREFERENCES,
        THEME_DARK,
        THEME_LIGHT,
        THEME_SYSTEM,
        THEME_TOGGLE,
        PANE_SPLIT_H,
        PANE_SPLIT_V,
        PANE_CLOSE,
        PANE_SWAP,
        PANE_EQUALIZE,
        PANE_ZOOM,
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
        PREVIEW_WRAP,
        LOG_FOLLOW,
        LOG_COLUMNS,
        LOG_MARK,
        LOG_EARLIER,
        THUMBS_TOGGLE,
        THUMBS_SIZE,
        THUMBS_CLEAR,
        VIDEO_PLAY,
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
        FILE_DELETE,
        FILE_DELETE_PERMANENT,
        EDIT_UNDO,
        FILE_CLIP_COPY,
        FILE_CLIP_CUT,
        FILE_CLIP_PASTE,
        WS_NEW,
        WS_CLOSE,
        WS_SAVE,
        WS_RENAME,
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
    let label = spec.label.to_string();
    let enabled = (spec.enabled)(session);
    let mut button = egui::Button::new(label);
    if let Some(shortcut) = spec.shortcut {
        // Right-aligned, weak-styled after the label (egui menu convention).
        button = button.shortcut_text(shortcut);
    }
    if ui.add_enabled(enabled, button).clicked() {
        dispatch(session, dialogs, id);
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

/// The top menu bar (File / Panes / View / Help).
pub fn menu_bar(ui: &mut egui::Ui, session: &mut Session, dialogs: &mut Dialogs) {
    egui::menu::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, session, dialogs, FILE_COPY);
            item(ui, session, dialogs, FILE_MOVE);
            item(ui, session, dialogs, FILE_MKDIR);
            item(ui, session, dialogs, FILE_NEWFILE);
            item(ui, session, dialogs, FILE_RENAME);
            ui.separator();
            item(ui, session, dialogs, FILE_DELETE);
            item(ui, session, dialogs, FILE_DELETE_PERMANENT);
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
        });
        ui.menu_button("Go", |ui| {
            item(ui, session, dialogs, NAV_GOTO);
            ui.separator();
            item(ui, session, dialogs, NAV_UP);
            item(ui, session, dialogs, NAV_BACK);
            item(ui, session, dialogs, NAV_FORWARD);
            item(ui, session, dialogs, NAV_REFRESH);
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
            check_item(ui, session, dialogs, VIEW_HIDDEN, session.show_hidden);
            check_item(ui, session, dialogs, VIEW_LINKS, session.show_link_targets);
            ui.separator();
            item(ui, session, dialogs, VIEW_PREVIEW);
            ui.menu_button("Preview", |ui| {
                item(ui, session, dialogs, PREVIEW_PRETTY);
                item(ui, session, dialogs, PREVIEW_WRAP);
                ui.separator();
                item(ui, session, dialogs, THUMBS_TOGGLE);
                item(ui, session, dialogs, THUMBS_SIZE);
                item(ui, session, dialogs, THUMBS_CLEAR);
                item(ui, session, dialogs, VIDEO_PLAY);
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
        });
        ui.menu_button("Help", |ui| {
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
    let typing = dialogs.is_open() || ctx.text_edit_focused();
    // Most modifiers first, so Cmd+Backspace wins over plain Backspace.
    let mut table = shortcuts();
    table.sort_by_key(|(_, sc)| std::cmp::Reverse(modifier_count(&sc.modifiers)));
    // A focused terminal owns the keyboard: Ctrl+<key>, Backspace, Tab, F-keys and friends
    // belong to the shell. Only Cmd/Ctrl+Shift chords (and macOS Cmd) stay app shortcuts.
    let terminal = !dialogs.is_open() && session.focused_kind() == Some(PaneKind::Terminal);
    for (id, shortcut) in table {
        if terminal && !terminal_shortcut(&shortcut) {
            continue;
        }
        if typing && (id.starts_with("term.") || id.starts_with("pane.resize.")) {
            continue;
        }
        if typing && (id.starts_with("file.") || id == EDIT_UNDO || shortcut.modifiers.is_none()) {
            continue;
        }
        let consumed = ctx.input_mut(|input| input.consume_shortcut(&shortcut));
        if consumed {
            dispatch(session, dialogs, id);
            fired.push(id);
            if id == PANE_SWAP {
                // Cancel the focus move egui already staged for this `Tab`.
                ctx.memory_mut(|mem| mem.move_focus(egui::FocusDirection::None));
            }
        }
    }
    if !terminal && !typing {
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
    if !terminal && !typing && !dialogs.is_open() {
        use egui::Key::*;
        let digits = [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9, Num0];
        for (ix, key) in digits.into_iter().enumerate() {
            let sc = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key);
            if ctx.input_mut(|input| input.consume_shortcut(&sc)) {
                session.dispatch_or_notice("workspace.switch", &serde_json::json!({ "n": ix + 1 }));
                fired.push("workspace.switch");
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
            let is_alias =
                id == FILE_DELETE && (expected == "Delete" || expected == "Ctrl+Backspace");
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
            (Key::C, ctrl),
            (Key::D, ctrl),
            (Key::R, ctrl),
        ] {
            assert!(
                chord(&ctx, &mut session, key, m).is_empty(),
                "{key:?} {m:?}"
            );
        }
        let paste = chord(&ctx, &mut session, Key::V, ctrl_shift);
        assert_eq!(paste, vec![TERM_PASTE]);
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
