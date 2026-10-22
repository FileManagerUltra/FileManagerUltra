//! Theme commands: dark, light, system, toggle.

use crate::prefs::ThemePreference;
use crate::registry::{CommandSpec, Params};

/// `view.theme.dark`
pub fn dark() -> CommandSpec {
    CommandSpec {
        id: "view.theme.dark",
        label: "Dark",
        menu: &["View", "Theme", "Dark"],
        shortcut: None,
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.prefs.theme = ThemePreference::Dark;
            Ok(())
        },
    }
}

/// `view.theme.light`
pub fn light() -> CommandSpec {
    CommandSpec {
        id: "view.theme.light",
        label: "Light",
        menu: &["View", "Theme", "Light"],
        shortcut: None,
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.prefs.theme = ThemePreference::Light;
            Ok(())
        },
    }
}

/// `view.theme.system`
pub fn system() -> CommandSpec {
    CommandSpec {
        id: "view.theme.system",
        label: "System",
        menu: &["View", "Theme", "System"],
        shortcut: None,
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.prefs.theme = ThemePreference::System;
            Ok(())
        },
    }
}

/// `view.theme.toggle` — cycles Dark → Light → Dark; System becomes Dark.
pub fn toggle() -> CommandSpec {
    CommandSpec {
        id: "view.theme.toggle",
        label: "Toggle dark / light",
        menu: &["View", "Theme"],
        shortcut: Some("Ctrl+Shift+T"),
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.prefs.theme = match session.prefs.theme {
                ThemePreference::Dark => ThemePreference::Light,
                ThemePreference::Light | ThemePreference::System => ThemePreference::Dark,
            };
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    #[test]
    fn toggle_cycles_dark_and_light() {
        let mut session = Session::default();
        assert_eq!(session.prefs.theme, ThemePreference::System);
        (toggle().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::Dark);
        (toggle().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::Light);
        (toggle().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::Dark);
    }

    #[test]
    fn explicit_theme_commands_set_the_preference() {
        let mut session = Session::default();
        (light().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::Light);
        (system().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::System);
        (dark().run)(&mut session, &Params::Null).expect("run");
        assert_eq!(session.prefs.theme, ThemePreference::Dark);
    }
}
