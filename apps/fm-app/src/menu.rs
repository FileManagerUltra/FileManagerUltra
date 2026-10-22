//! Menu catalogue and keyboard shortcuts.
//!
//! Every id referenced here MUST exist in the engine registry — asserted by the tests
//! in this crate and by `cargo xtask brand-check`. The UI never calls operations
//! directly: it only dispatches command ids.

use eframe::egui;

use fm_engine::{Params, Session, ThemePreference};

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
    ]
}

/// Dispatch helper: run a command; a failure must never crash the frame.
fn dispatch(session: &mut Session, id: &str) {
    if let Err(err) = session.dispatch(id, &Params::Null) {
        eprintln!("command {id} failed: {err}");
    }
}

fn item(ui: &mut egui::Ui, session: &mut Session, id: &str) {
    let Some(spec) = session.registry.get(id) else {
        return;
    };
    let label = spec.label.to_string();
    let enabled = (spec.enabled)(session);
    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
        dispatch(session, id);
        ui.close();
    }
}

fn theme_radio(ui: &mut egui::Ui, session: &mut Session, id: &str, expected: ThemePreference) {
    let selected = session.prefs.theme == expected;
    let label = match expected {
        ThemePreference::System => "System",
        ThemePreference::Dark => "Dark",
        ThemePreference::Light => "Light",
    };
    if ui.selectable_label(selected, label).clicked() {
        dispatch(session, id);
    }
}

/// The top menu bar (File / Panes / View / Help).
pub fn menu_bar(ui: &mut egui::Ui, session: &mut Session) {
    egui::menu::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            item(ui, session, QUIT);
        });
        ui.menu_button("Panes", |ui| {
            item(ui, session, PANE_SPLIT_H);
            item(ui, session, PANE_SPLIT_V);
            ui.separator();
            item(ui, session, PANE_SWAP);
            item(ui, session, PANE_FOCUS_NEXT);
            ui.separator();
            item(ui, session, PANE_CLOSE);
        });
        ui.menu_button("View", |ui| {
            ui.menu_button("Theme", |ui| {
                theme_radio(ui, session, THEME_SYSTEM, ThemePreference::System);
                theme_radio(ui, session, THEME_DARK, ThemePreference::Dark);
                theme_radio(ui, session, THEME_LIGHT, ThemePreference::Light);
                ui.separator();
                item(ui, session, THEME_TOGGLE);
            });
            ui.menu_button("Sort", |ui| {
                item(ui, session, SORT_NAME);
                item(ui, session, SORT_SIZE);
                item(ui, session, SORT_MTIME);
                item(ui, session, SORT_TYPE);
                ui.separator();
                item(ui, session, SORT_DIRS_FIRST);
            });
        });
        ui.menu_button("Help", |ui| {
            item(ui, session, ABOUT);
        });
    });
}

/// Apply keyboard shortcuts; returns the ids that fired.
pub fn consume_shortcuts(ctx: &egui::Context, session: &mut Session) -> Vec<&'static str> {
    let mut fired = Vec::new();
    for (id, shortcut) in shortcuts() {
        let consumed = ctx.input_mut(|input| input.consume_shortcut(&shortcut));
        if consumed {
            dispatch(session, id);
            fired.push(id);
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
        dispatch(&mut session, THEME_TOGGLE);
        assert_eq!(session.prefs.theme, ThemePreference::Light);
        dispatch(&mut session, ABOUT);
        assert!(session.about_open);
        dispatch(&mut session, QUIT);
        assert!(!session.running);
    }

    #[test]
    fn pane_commands_dispatch_through_the_session() {
        let mut session = Session::new().expect("session");
        dispatch(&mut session, PANE_SPLIT_H);
        assert_eq!(session.layout.panes().len(), 2);
        dispatch(&mut session, PANE_FOCUS_NEXT);
        dispatch(&mut session, PANE_SWAP);
        dispatch(&mut session, PANE_CLOSE);
        assert_eq!(session.layout.panes().len(), 1);
    }
}
