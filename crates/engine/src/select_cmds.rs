//! Selection and quick-filter commands for the focused Files pane: Select All / None /
//! Invert, Select by Pattern and the listing's type-to-filter bar.
//!
//! They work on the *visible* rows ([`Session::pane_view`]: hidden files and the quick
//! filter respected), so a selection never contains something the user cannot see.

use std::collections::BTreeSet;

use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::glob::{Glob, MAX_PATTERN_LEN};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Visible names of the focused Files pane in display order.
fn visible_names(s: &mut Session) -> Result<(fm_layout::PaneId, Vec<String>), FmError> {
    let pane = s.layout.focused();
    if !s.focused_is_files() {
        return Err(FmError::InvalidParams(
            "the focused pane does not list files".to_string(),
        ));
    }
    let names = s
        .pane_view(pane)
        .map(|(entries, ix)| {
            ix.order
                .iter()
                .filter_map(|i| entries.get(*i as usize).map(|e| e.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    Ok((pane, names))
}

fn with_listing(
    s: &mut Session,
    params: &Params,
    f: impl FnOnce(&mut fm_layout::ListingState, &[String]),
) -> Result<(), FmError> {
    object(params)?;
    let (pane, names) = visible_names(s)?;
    if let Some(state) = s.layout.state_mut(pane) {
        state.tree.active = false;
        f(&mut state.listing, &names);
    }
    Ok(())
}

fn files_focused(s: &Session) -> bool {
    s.focused_is_files()
}

pub fn select_all() -> CommandSpec {
    CommandSpec {
        id: "edit.select_all",
        label: "Select All",
        menu: &["Edit", "Select All"],
        shortcut: Some("Ctrl+A"),
        params: "none",
        enabled: files_focused,
        run: |s, params| with_listing(s, params, |l, names| l.select_all(names)),
    }
}

pub fn select_none() -> CommandSpec {
    CommandSpec {
        id: "edit.select_none",
        label: "Select None",
        menu: &["Edit", "Select None"],
        shortcut: Some("Ctrl+Shift+A"),
        params: "none",
        enabled: files_focused,
        run: |s, params| with_listing(s, params, |l, _| l.clear_selection()),
    }
}

pub fn invert() -> CommandSpec {
    CommandSpec {
        id: "edit.invert_selection",
        label: "Invert Selection",
        menu: &["Edit", "Invert Selection"],
        shortcut: Some("Ctrl+Shift+I"),
        params: "none",
        enabled: files_focused,
        run: |s, params| with_listing(s, params, |l, names| l.invert(names)),
    }
}

/// `edit.select_pattern` — `*.jpg; *.png` style; `mode` adds (default), removes, or
/// replaces the selection.
pub fn select_pattern() -> CommandSpec {
    CommandSpec {
        id: "edit.select_pattern",
        label: "Select by Pattern…",
        menu: &["Edit", "Select by Pattern…"],
        shortcut: Some("Ctrl+Shift+S"),
        params: "pattern: string (* ? [a-z] alternatives split by ;), mode?: add|remove|replace",
        enabled: files_focused,
        run: |s, params| {
            let map = object(params)?;
            let text = map
                .and_then(|m| m.get("pattern"))
                .and_then(Params::as_str)
                .ok_or_else(|| FmError::InvalidParams("pattern: string is required".into()))?;
            let glob = Glob::parse(text)?;
            let mode = match map.and_then(|m| m.get("mode")) {
                None => "add",
                Some(v) => match v.as_str() {
                    Some(m @ ("add" | "remove" | "replace")) => m,
                    _ => {
                        return Err(FmError::InvalidParams(
                            "mode must be add, remove or replace".into(),
                        ));
                    }
                },
            };
            let mut matched = 0;
            with_listing(s, &Params::Null, |l, names| {
                if mode == "replace" {
                    l.clear_selection();
                }
                matched = l.select_matching(names, mode != "remove", |n| glob.matches(n));
            })?;
            let noun = if matched == 1 { "item" } else { "items" };
            s.info = Some((
                format!("{matched} {noun} match “{text}”"),
                std::time::Instant::now(),
            ));
            Ok(())
        },
    }
}

/// `view.filter` — show only names containing `text` (or matching it, with wildcards).
/// Without `text` it opens the filter bar and focuses it.
pub fn filter() -> CommandSpec {
    CommandSpec {
        id: "view.filter",
        label: "Filter Listing",
        menu: &["View", "Filter Listing"],
        // Typing in a Files listing opens it (see fm-ui listing); Ctrl+F is Find Files.
        shortcut: None,
        params: "text?: string (a substring or a pattern with * ? [..]), focus?: bool (put the cursor in the filter field)",
        enabled: files_focused,
        run: |s, params| {
            let map = object(params)?;
            let text = match map.and_then(|m| m.get("text")) {
                None => None,
                Some(v) => {
                    let t = v
                        .as_str()
                        .ok_or_else(|| FmError::InvalidParams("text must be a string".into()))?;
                    if t.len() > MAX_PATTERN_LEN {
                        return Err(FmError::InvalidParams("filter too long".into()));
                    }
                    Some(t.to_string())
                }
            };
            let focus = match map.and_then(|m| m.get("focus")) {
                None => false,
                Some(v) => v
                    .as_bool()
                    .ok_or_else(|| FmError::InvalidParams("focus must be a boolean".into()))?,
            };
            let pane = s.layout.focused();
            if !s.focused_is_files() {
                return Err(FmError::InvalidParams(
                    "the focused pane does not list files".to_string(),
                ));
            }
            if let Some(state) = s.layout.state_mut(pane) {
                match text {
                    Some(t) => {
                        state.listing.filter = Some(t);
                        state.listing.filter_focus |= focus;
                    }
                    None => {
                        state.listing.filter.get_or_insert_with(String::new);
                        state.listing.filter_focus = true;
                    }
                }
                state.listing.focused_row = 0;
            }
            // Keep the selection to rows that are still visible.
            let (_, names) = visible_names(s)?;
            let visible: BTreeSet<String> = names.into_iter().collect();
            if let Some(state) = s.layout.state_mut(pane) {
                state.listing.prune(&visible);
            }
            Ok(())
        },
    }
}

pub fn filter_clear() -> CommandSpec {
    CommandSpec {
        id: "view.filter.clear",
        label: "Clear Filter",
        menu: &["View", "Clear Filter"],
        shortcut: None,
        params: "none",
        enabled: |s| {
            s.layout
                .state(s.layout.focused())
                .is_some_and(|p| p.kind.is_files() && p.listing.filter.is_some())
        },
        run: |s, params| {
            object(params)?;
            let pane = s.layout.focused();
            if let Some(state) = s.layout.state_mut(pane) {
                state.listing.filter = None;
                state.listing.filter_focus = false;
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        select_all(),
        select_none(),
        invert(),
        select_pattern(),
        filter(),
        filter_clear(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::VfsUri;
    use serde_json::json;

    fn session_in(dir: &std::path::Path) -> Session {
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(dir.to_str().expect("utf8")).expect("uri");
        let pane = s.layout.focused();
        if let Some(st) = s.layout.state_mut(pane) {
            st.navigate(uri.clone());
        }
        s.submit_read(uri).expect("read");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !s.pane_view_ready(pane) && std::time::Instant::now() < deadline {
            s.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        s
    }

    fn selected(s: &Session) -> Vec<String> {
        s.layout
            .state(s.layout.focused())
            .map(|p| p.listing.selection().cloned().collect())
            .unwrap_or_default()
    }

    #[test]
    fn select_invert_pattern_and_filter_follow_the_visible_rows() {
        let tmp = tempfile::tempdir().expect("tmp");
        for n in ["a.jpg", "b.PNG", "c.txt", ".hidden.jpg"] {
            std::fs::write(tmp.path().join(n), b"x").expect("write");
        }
        let mut s = session_in(tmp.path());
        s.dispatch("edit.select_all", &Params::Null).expect("all");
        assert_eq!(selected(&s), vec!["a.jpg", "b.PNG", "c.txt"]);
        s.dispatch("edit.select_none", &Params::Null).expect("none");
        assert!(selected(&s).is_empty());
        s.dispatch("edit.select_pattern", &json!({"pattern": "*.jpg;*.png"}))
            .expect("pattern");
        assert_eq!(selected(&s), vec!["a.jpg", "b.PNG"]);
        s.dispatch("edit.invert_selection", &Params::Null)
            .expect("invert");
        assert_eq!(selected(&s), vec!["c.txt"]);
        s.dispatch(
            "edit.select_pattern",
            &json!({"pattern": "*.txt", "mode": "remove"}),
        )
        .expect("remove");
        assert!(selected(&s).is_empty());

        s.dispatch("edit.select_all", &Params::Null).expect("all");
        s.dispatch("view.filter", &json!({"text": "jp"}))
            .expect("filter");
        assert_eq!(selected(&s), vec!["a.jpg"], "filter prunes the selection");
        let pane = s.layout.focused();
        let (_, ix) = s.pane_view(pane).expect("view");
        assert_eq!(ix.order.len(), 1);
        s.dispatch("view.filter", &json!({"text": "*.{"}))
            .expect("odd");
        s.dispatch("view.filter.clear", &Params::Null)
            .expect("clear");
        let (_, ix) = s.pane_view(pane).expect("view");
        assert_eq!(ix.order.len(), 3);
        s.dispatch("view.filter", &Params::Null).expect("open bar");
        let l = &s.layout.state(pane).expect("pane").listing;
        assert!(l.filter.as_deref() == Some("") && l.filter_focus);
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("edit.select_pattern", json!({})),
            ("edit.select_pattern", json!({"pattern": ""})),
            ("edit.select_pattern", json!({"pattern": "*", "mode": "x"})),
            ("edit.select_pattern", json!({"pattern": 4})),
            ("view.filter", json!({"text": 1})),
            (
                "view.filter",
                json!({"text": "x".repeat(MAX_PATTERN_LEN + 1)}),
            ),
            ("edit.select_all", json!([1])),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
