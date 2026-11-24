//! Application commands: quit, about.

use crate::registry::{CommandSpec, Params};

/// `app.quit` — ask the shell to stop (the eframe loop observes `running == false`); with
/// unsaved workspace changes it raises the confirmation first.
pub fn quit() -> CommandSpec {
    CommandSpec {
        id: "app.quit",
        label: "Quit",
        menu: &["File", "Quit"],
        shortcut: Some("Ctrl+Q"),
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.request_quit();
            Ok(())
        },
    }
}

impl crate::session::Session {
    /// Work that quitting would stop, one readable line each.
    pub fn background_work(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .fileops
            .jobs()
            .iter()
            .filter(|j| j.state.is_running())
            .map(|j| j.label.clone())
            .collect();
        for st in self.sync.states.values() {
            let name = if st.profile.name.trim().is_empty() {
                "Unnamed sync"
            } else {
                st.profile.name.as_str()
            };
            if st.busy() {
                out.push(format!("Synchronizing \u{201c}{name}\u{201d}"));
            } else if st.watching() {
                out.push(format!("Keeping \u{201c}{name}\u{201d} in sync"));
            }
        }
        out
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

/// `app.licenses` — raise the Third-party Licences window flag (the UI renders and clears it).
pub fn licenses() -> CommandSpec {
    CommandSpec {
        id: "app.licenses",
        label: "Third-party Licences",
        menu: &["Help", "Third-party Licences"],
        shortcut: None,
        params: "none",
        enabled: |_session| true,
        run: |session, _params: &Params| {
            session.licenses_open = true;
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
