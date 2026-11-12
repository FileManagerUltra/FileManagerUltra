//! Pane topology commands: split, close, swap, focus-next.

use fm_layout::Axis;
use fm_vfs::FmError;

use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// `pane.split.horizontal` — split the focused pane into left | right.
pub fn split_horizontal() -> CommandSpec {
    CommandSpec {
        id: "pane.split.horizontal",
        label: "Split Right",
        menu: &["Panes", "Split Right"],
        shortcut: Some("Ctrl+Shift+E"),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| split(s, Axis::Horizontal),
    }
}

/// `pane.split.vertical` — split the focused pane into top / bottom.
pub fn split_vertical() -> CommandSpec {
    CommandSpec {
        id: "pane.split.vertical",
        label: "Split Down",
        menu: &["Panes", "Split Down"],
        shortcut: Some("Ctrl+Shift+O"),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| split(s, Axis::Vertical),
    }
}

fn split(s: &mut Session, axis: Axis) -> Result<(), FmError> {
    use fm_layout::PaneKind;
    let focused = s.layout.focused();
    let new_id = s.layout.split(focused, axis, 0.5)?;
    // Splitting the Media or a Preview pane opens a Files pane beside it.
    let converted = s
        .layout
        .state(new_id)
        .is_some_and(|state| matches!(state.kind, PaneKind::Media | PaneKind::Preview));
    if converted {
        // Its stored folder is whatever the pane last listed, possibly a remote one that
        // is no longer connected: reading that would ask to log in. Keep a local or live
        // location, else start at home.
        let keep = s.layout.state(new_id).is_some_and(|state| {
            let virtual_root = matches!(
                state.tree.root.scheme(),
                fm_vfs::Scheme::Network | fm_vfs::Scheme::Servers
            );
            !virtual_root
                && (state.location.scheme() == fm_vfs::Scheme::File
                    || s.backends.get_for(&state.location).is_some())
        });
        if let Some(state) = s.layout.state_mut(new_id) {
            if keep {
                state.kind = PaneKind::Files;
            } else {
                *state = fm_layout::PaneState::new_rooted(
                    crate::workspace::home_location(),
                    fm_vfs::VfsUri::root_file(),
                );
            }
        }
    }
    // A new listing pane inherits the location; make sure it is loaded (no-op when the
    // shared cache already has it). Other kinds list nothing.
    if let Some(state) = s.layout.state(new_id).filter(|state| state.kind.is_files()) {
        let loc = state.location.clone();
        s.ensure_read(loc)?;
    }
    Ok(())
}

/// `pane.close` — close `pane` (default: the focused one); disabled on the last pane.
pub fn close() -> CommandSpec {
    CommandSpec {
        id: "pane.close",
        label: "Close Pane",
        menu: &["Panes", "Close"],
        shortcut: None,
        params: "pane?: id",
        enabled: |s| s.layout.panes().len() > 1,
        run: |s, params: &Params| {
            if s.layout.panes().len() <= 1 {
                return Ok(()); // defensive no-op; the menu is greyed out
            }
            let target = match crate::file_cmds::object(params)?.and_then(|m| m.get("pane")) {
                Some(value) => value
                    .as_u64()
                    .ok_or_else(|| FmError::InvalidParams("pane: pane id".to_string()))?,
                None => s.layout.focused(),
            };
            s.layout.close(target)?;
            s.drop_terminal(target);
            s.media_visibility_changed();
            Ok(())
        },
    }
}

/// `pane.swap` — commander-style Tab: swap with the previously focused pane.
pub fn swap() -> CommandSpec {
    CommandSpec {
        id: "pane.swap",
        label: "Swap Panes",
        menu: &["Panes", "Swap"],
        shortcut: Some("Tab"),
        params: "none",
        enabled: |s| s.layout.focus.previous.is_some(),
        run: |s, _p: &Params| {
            if s.layout.focus.previous.is_none() {
                return Ok(());
            }
            s.layout.swap_focused_with_previous()
        },
    }
}

/// `pane.focus.next` — cycle focus through the leaves.
pub fn focus_next() -> CommandSpec {
    CommandSpec {
        id: "pane.focus.next",
        label: "Next Pane",
        menu: &["Panes", "Next Pane"],
        shortcut: None,
        params: "none",
        enabled: |s| s.layout.panes().len() > 1,
        run: |s, _p: &Params| {
            if s.layout.panes().len() > 1 {
                s.layout.focus_next();
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        split_horizontal(),
        split_vertical(),
        close(),
        swap(),
        focus_next(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Params;

    #[test]
    fn split_adds_a_pane_and_focus_moves() {
        let mut s = Session::default();
        assert_eq!(s.layout.panes().len(), 1);
        (split_horizontal().run)(&mut s, &Params::Null).expect("split");
        assert_eq!(s.layout.panes().len(), 2);
        let focused = s.layout.focused();
        assert_ne!(focused, 1);
        (split_vertical().run)(&mut s, &Params::Null).expect("split");
        assert_eq!(s.layout.panes().len(), 3);
    }

    #[test]
    fn close_is_a_noop_on_the_last_pane_and_works_otherwise() {
        let mut s = Session::default();
        assert!(!(close().enabled)(&s));
        (close().run)(&mut s, &Params::Null).expect("noop");
        assert_eq!(s.layout.panes().len(), 1);
        (split_horizontal().run)(&mut s, &Params::Null).expect("split");
        assert!((close().enabled)(&s));
        (close().run)(&mut s, &Params::Null).expect("close");
        assert_eq!(s.layout.panes().len(), 1);
        assert_eq!(s.layout.focused(), 1);
    }

    #[test]
    fn swap_needs_a_previous_pane() {
        let mut s = Session::default();
        assert!(!(swap().enabled)(&s));
        (swap().run)(&mut s, &Params::Null).expect("noop");
        (split_horizontal().run)(&mut s, &Params::Null).expect("split");
        assert!((swap().enabled)(&s));
        let panes_before = s.layout.panes();
        (swap().run)(&mut s, &Params::Null).expect("swap");
        assert_eq!(s.layout.panes().len(), 2);
        assert_ne!(s.layout.panes(), panes_before, "positions exchanged");
        assert_eq!(s.layout.focused(), 1, "focus follows the partner");
    }

    #[test]
    fn focus_next_cycles_and_wraps() {
        let mut s = Session::default();
        (split_horizontal().run)(&mut s, &Params::Null).expect("split");
        let second = s.layout.focused();
        (focus_next().run)(&mut s, &Params::Null).expect("next");
        assert_eq!(s.layout.focused(), 1, "wrapped to the first leaf");
        (focus_next().run)(&mut s, &Params::Null).expect("next");
        assert_eq!(s.layout.focused(), second);
    }

    #[test]
    fn splitting_a_preview_opens_a_files_pane_without_asking_to_log_in() {
        let mut s = Session::default();
        let pane = s.layout.focused();
        // A pane that once showed an SFTP server, now a Preview; nothing is connected.
        let remote = fm_vfs::VfsUri::parse("sftp://alice@server.test/home/alice").expect("uri");
        {
            let state = s.layout.state_mut(pane).expect("pane");
            *state = fm_layout::PaneState::new(remote);
            state.kind = fm_layout::PaneKind::Preview;
        }
        (split_horizontal().run)(&mut s, &Params::Null).expect("split");
        let new = s.layout.focused();
        let state = s.layout.state(new).expect("new pane");
        assert_eq!(
            state.kind,
            fm_layout::PaneKind::Files,
            "not a second preview"
        );
        assert_eq!(state.location, crate::workspace::home_location());
        assert!(s.remote.sftp_prompt.is_none(), "no SFTP login dialog");
        assert_eq!(
            s.layout.state(pane).expect("preview").kind,
            fm_layout::PaneKind::Preview,
            "the preview itself stays"
        );

        // A Preview that last showed a local folder keeps it.
        let mut s = Session::default();
        let pane = s.layout.focused();
        let docs = fm_vfs::VfsUri::parse("/tmp").expect("uri");
        s.layout
            .state_mut(pane)
            .expect("pane")
            .navigate(docs.clone());
        s.layout.state_mut(pane).expect("pane").kind = fm_layout::PaneKind::Preview;
        (split_vertical().run)(&mut s, &Params::Null).expect("split");
        let state = s.layout.state(s.layout.focused()).expect("new pane");
        assert_eq!(
            (state.kind, &state.location),
            (fm_layout::PaneKind::Files, &docs)
        );
    }

    #[test]
    fn split_of_unknown_pane_errors_without_panic() {
        let mut s = Session::default();
        // Corrupt the focused id; split must fail, not panic.
        s.layout.focus.focused = 999;
        let err = (split_horizontal().run)(&mut s, &Params::Null);
        assert!(err.is_err());
    }
}
