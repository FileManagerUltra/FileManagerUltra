//! Menu catalogue and keyboard shortcuts.
//!
//! Every id referenced here MUST exist in the engine registry — asserted by the tests
//! in this crate and by `cargo xtask brand-check`. The UI never calls operations
//! directly: it only dispatches command ids.

use eframe::egui;

use fm_engine::{Params, Session, ThemePreference};
use fm_ui::dialogs::{Dialogs, dialog_for};

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
pub const PANE_FOCUS_NEXT: &str = "pane.focus.next";
pub const NAV_UP: &str = "nav.up";
pub const NAV_BACK: &str = "nav.back";
pub const NAV_FORWARD: &str = "nav.forward";
pub const NAV_REFRESH: &str = "nav.refresh";
pub const SORT_NAME: &str = "view.sort.name";
pub const SORT_SIZE: &str = "view.sort.size";
pub const SORT_MTIME: &str = "view.sort.mtime";
pub const SORT_TYPE: &str = "view.sort.type";
pub const SORT_DIRS_FIRST: &str = "view.sort.dirs_first";
pub const FILE_COPY: &str = "file.copy";
pub const FILE_MOVE: &str = "file.move";
pub const FILE_MKDIR: &str = "file.mkdir";
pub const FILE_RENAME: &str = "file.rename";
pub const FILE_DELETE: &str = "file.delete";
pub const FILE_DELETE_PERMANENT: &str = "file.delete_permanent";
pub const EDIT_UNDO: &str = "edit.undo";

/// Every command id referenced by the menus or the shortcut table.
/// (Used by the tests and by `cargo xtask brand-check`'s registry cross-check.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn all_ids() -> &'static [&'static str] {
    &[
        QUIT,
        ABOUT,
        THEME_DARK,
        THEME_LIGHT,
        THEME_SYSTEM,
        THEME_TOGGLE,
        PANE_SPLIT_H,
        PANE_SPLIT_V,
        PANE_CLOSE,
        PANE_SWAP,
        PANE_FOCUS_NEXT,
        NAV_UP,
        NAV_BACK,
        NAV_FORWARD,
        NAV_REFRESH,
        SORT_NAME,
        SORT_SIZE,
        SORT_MTIME,
        SORT_TYPE,
        SORT_DIRS_FIRST,
        FILE_COPY,
        FILE_MOVE,
        FILE_MKDIR,
        FILE_RENAME,
        FILE_DELETE,
        FILE_DELETE_PERMANENT,
        EDIT_UNDO,
    ]
}

/// Keyboard shortcuts, id → egui binding. Display strings live on the CommandSpecs
/// (`"Ctrl+Q"`, `"Alt+Left"`…); the test in this module checks they agree.
pub fn shortcuts() -> Vec<(&'static str, egui::KeyboardShortcut)> {
    use egui::{Key, Modifiers};
    vec![
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
            SORT_DIRS_FIRST,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::H),
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
            FILE_RENAME,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F2),
        ),
        (
            FILE_DELETE,
            egui::KeyboardShortcut::new(Modifiers::NONE, Key::F8),
        ),
        (
            FILE_DELETE_PERMANENT,
            egui::KeyboardShortcut::new(Modifiers::SHIFT, Key::Delete),
        ),
        (
            EDIT_UNDO,
            egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Z),
        ),
    ]
}

/// Run a command; ids that need input first open their dialog instead. A failure becomes the
/// visible notice and never crashes the frame.
fn dispatch(session: &mut Session, dialogs: &mut Dialogs, id: &str) {
    if let Some(dialog) = dialog_for(id, session) {
        dialogs.request(dialog);
        return;
    }
    if let Err(err) = session.dispatch(id, &Params::Null) {
        session.notice = Some(err.to_string());
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
            item(ui, session, dialogs, FILE_RENAME);
            ui.separator();
            item(ui, session, dialogs, FILE_DELETE);
            item(ui, session, dialogs, FILE_DELETE_PERMANENT);
            ui.separator();
            item(ui, session, dialogs, QUIT);
        });
        ui.menu_button("Edit", |ui| {
            item(ui, session, dialogs, EDIT_UNDO);
        });
        ui.menu_button("Panes", |ui| {
            item(ui, session, dialogs, PANE_SPLIT_H);
            item(ui, session, dialogs, PANE_SPLIT_V);
            ui.separator();
            item(ui, session, dialogs, PANE_SWAP);
            item(ui, session, dialogs, PANE_FOCUS_NEXT);
            ui.separator();
            item(ui, session, dialogs, PANE_CLOSE);
        });
        ui.menu_button("View", |ui| {
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

/// Apply keyboard shortcuts; returns the ids that fired.
///
/// Call this at the top of the frame, before any widget renders: `Tab` is also
/// egui's focus-traversal key, and its `focus_direction` is committed at
/// `begin_pass`. Consuming the shortcut here (and cancelling the direction for
/// `pane.swap`) keeps `Tab` swapping panes instead of tabbing through widgets.
pub fn consume_shortcuts(
    ctx: &egui::Context,
    session: &mut Session,
    dialogs: &mut Dialogs,
) -> Vec<&'static str> {
    let mut fired = Vec::new();
    // File shortcuts must not fire while a dialog or a text field owns the keyboard.
    let typing = dialogs.is_open() || ctx.egui_wants_keyboard_input();
    for (id, shortcut) in shortcuts() {
        if typing && (id.starts_with("file.") || id == EDIT_UNDO) {
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
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert_eq!(display, expected, "shortcut display mismatch for {id}");
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
            Key::T => "T",
            Key::H => "H",
            Key::R => "R",
            Key::Z => "Z",
            Key::F2 => "F2",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F7 => "F7",
            Key::F8 => "F8",
            Key::Delete => "Delete",
            Key::Tab => "Tab",
            Key::Backspace => "Backspace",
            Key::ArrowLeft => "Left",
            Key::ArrowRight => "Right",
            _ => "?",
        }
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
