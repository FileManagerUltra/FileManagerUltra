//! View-wide toggles that are not per-pane sort state.

use fm_layout::PaneId;
use fm_vfs::FmError;

use crate::registry::{CommandSpec, Params};
use crate::session::Session;

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
            let panes = s.layout.panes();
            // Row numbers shift when entries appear or vanish, so follow the cursor by name.
            let before: Vec<_> = panes.iter().map(|p| (*p, cursor_context(s, *p))).collect();
            s.show_hidden = !s.show_hidden;
            for (pane, context) in before {
                s.invalidate_view(pane);
                restore_cursor(s, pane, context);
            }
            Ok::<(), FmError>(())
        },
    }
}

/// Visible names in display order plus the cursor row, captured before the view changes.
type CursorContext = Option<(Vec<String>, usize)>;

fn cursor_context(s: &mut Session, pane: PaneId) -> CursorContext {
    let row = s.layout.state(pane)?.listing.focused_row;
    let (entries, ix) = s.pane_view(pane)?;
    let names = ix
        .order
        .iter()
        .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
        .collect();
    Some((names, row))
}

/// Put the cursor back on the entry it was on, or the nearest one that is still visible,
/// and drop selected entries that are no longer shown.
fn restore_cursor(s: &mut Session, pane: PaneId, context: CursorContext) {
    let Some((old, row)) = context else {
        // No loaded view to compare against: a selection must never include unseen rows.
        if let Some(state) = s.layout.state_mut(pane) {
            state.listing.clear_selection();
            state.listing.focused_row = 0;
        }
        return;
    };
    let Some((entries, ix)) = s.pane_view(pane) else {
        return;
    };
    let new: Vec<String> = ix
        .order
        .iter()
        .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
        .collect();
    let row = row.min(old.len().saturating_sub(1));
    let after = old.get(row..).unwrap_or_default().iter();
    let before = old.get(..row).unwrap_or_default().iter().rev();
    let found = after
        .chain(before)
        .find_map(|name| new.iter().position(|n| n == name));
    let visible: std::collections::BTreeSet<String> = new.into_iter().collect();
    if let Some(state) = s.layout.state_mut(pane) {
        state.listing.prune(&visible);
        if let Some(row) = found {
            state.listing.focused_row = row;
        } else {
            state.listing.focused_row = 0;
        }
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
