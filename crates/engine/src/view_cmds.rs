//! View-wide toggles that are not per-pane sort state.

use fm_vfs::FmError;

use crate::registry::{CommandSpec, Params};

/// `view.hidden.toggle` — show or hide dot-files in every pane and tree.
pub fn toggle_hidden() -> CommandSpec {
    CommandSpec {
        id: "view.hidden.toggle",
        label: "Show Hidden Files",
        menu: &["View", "Show Hidden Files"],
        shortcut: Some(if cfg!(target_os = "macos") {
            "Ctrl+Shift+."
        } else {
            "Ctrl+H"
        }),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            s.show_hidden = !s.show_hidden;
            let panes = s.layout.panes();
            for pane in panes {
                // A selection must never include rows the user cannot see.
                if let Some(state) = s.layout.state_mut(pane) {
                    state.listing.clear_selection();
                    state.listing.focused_row = 0;
                }
                s.invalidate_view(pane);
            }
            Ok::<(), FmError>(())
        },
    }
}

/// `view.links.toggle` — show or hide `-> target` after symlink names.
pub fn toggle_link_targets() -> CommandSpec {
    CommandSpec {
        id: "view.links.toggle",
        label: "Show Link Targets",
        menu: &["View", "Show Link Targets"],
        shortcut: None,
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            s.show_link_targets = !s.show_link_targets;
            Ok::<(), FmError>(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![toggle_hidden(), toggle_link_targets()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn link_targets_toggle_flips_the_flag() {
        let mut s = Session::default();
        assert!(s.show_link_targets);
        (toggle_link_targets().run)(&mut s, &Params::Null).expect("toggle");
        assert!(!s.show_link_targets);
        (toggle_link_targets().run)(&mut s, &Params::Null).expect("toggle");
        assert!(s.show_link_targets);
    }

    #[test]
    fn toggle_flips_and_clears_selection() {
        let mut s = Session::default();
        assert!(!s.show_hidden);
        if let Some(st) = s.layout.state_mut(1) {
            st.listing.select_only(Some("a"), 0);
        }
        (toggle_hidden().run)(&mut s, &Params::Null).expect("toggle");
        assert!(s.show_hidden);
        assert_eq!(
            s.layout.state(1).map(|p| p.listing.selected_count()),
            Some(0)
        );
        (toggle_hidden().run)(&mut s, &Params::Null).expect("toggle");
        assert!(!s.show_hidden);
    }
}
