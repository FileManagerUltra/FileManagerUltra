//! Application commands: quit, about.

use crate::registry::{CommandSpec, Params};

/// `app.quit` — ask the shell to stop (the eframe loop observes `running == false`).
pub fn quit() -> CommandSpec {
    CommandSpec {
        id: "app.quit",
        label: "Quit",
        menu: &["File", "Quit"],
        shortcut: Some("Ctrl+Q"),
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.running = false;
            Ok(())
        },
    }
}

/// `app.about` — raise the About dialog flag (the UI renders and clears it).
pub fn about() -> CommandSpec {
    CommandSpec {
        id: "app.about",
        label: "About",
        menu: &["Help", "About"],
        shortcut: None,
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.about_open = true;
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::Session;

    #[test]
    fn quit_sets_running_false() {
        let mut session = Session::default();
        (quit().run)(&mut session, &Params::Null).expect("run");
        assert!(!session.running);
    }

    #[test]
    fn about_raises_the_flag() {
        let mut session = Session::default();
        (about().run)(&mut session, &Params::Null).expect("run");
        assert!(session.about_open);
    }

    #[test]
    fn commands_are_well_formed() {
        for spec in [quit(), about()] {
            assert!(!spec.id.is_empty());
            assert!(!spec.label.is_empty());
            assert!(!spec.menu.is_empty());
            assert!(!spec.params.is_empty());
        }
    }
}
