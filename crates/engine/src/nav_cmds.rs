//! Navigation commands: open, up, back, forward, refresh.

use fm_vfs::{FmError, VfsUri};

use crate::registry::{CommandSpec, Params};
use crate::session::Session;

fn focused_pane(s: &Session) -> Option<&fm_layout::PaneState> {
    s.layout.state(s.layout.focused())
}

/// Pull a `uri` string out of the untrusted params (`"…"` or `{"uri": "…"}`).
fn uri_param(params: &Params) -> Result<String, FmError> {
    if let Some(s) = params.as_str() {
        return Ok(s.to_string());
    }
    if let Some(map) = params.as_object()
        && let Some(s) = map.get("uri").and_then(|v| v.as_str())
    {
        return Ok(s.to_string());
    }
    Err(FmError::InvalidParams(
        "expected a uri string (or {\"uri\": string})".to_string(),
    ))
}

/// A navigation that did not start in the tree also expands the directory it lands on
/// (tree clicks toggle that node themselves).
fn expand_unless_tree_driven(pane: &mut fm_layout::PaneState) {
    if !pane.tree.active && pane.location != pane.tree.root {
        let loc = pane.location.clone();
        pane.tree.expand(loc);
    }
}

/// `nav.open` — navigate the focused pane to a uri.
pub fn open() -> CommandSpec {
    CommandSpec {
        id: "nav.open",
        label: "Open",
        menu: &["Go", "Open…"],
        shortcut: None,
        params: "uri: string (absolute path or file:// uri)",
        enabled: |_s| true,
        run: |s, params: &Params| {
            let raw = uri_param(params)?;
            let to = VfsUri::parse(&raw)
                .map_err(|_| FmError::invalid_uri(format!("nav.open: {raw}")))?;
            // No backend → error before committing the location (no ghost "loading").
            s.backends.require(&to)?;
            {
                let focused = s.layout.focused();
                let pane = s
                    .layout
                    .state_mut(focused)
                    .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
                pane.navigate(to.clone());
                expand_unless_tree_driven(pane);
            }
            s.note_visit(&to);
            s.ensure_read(to)?;
            Ok(())
        },
    }
}

/// `nav.up` — parent of the current location (no-op at the pane's tree root).
pub fn up() -> CommandSpec {
    CommandSpec {
        id: "nav.up",
        label: "Up",
        menu: &["Go", "Up"],
        shortcut: Some("Backspace"),
        params: "none",
        enabled: |s| focused_pane(s).is_some_and(|p| p.up_target().is_some()),
        run: |s, _p: &Params| {
            let target = match focused_pane(s).and_then(|p| p.up_target()) {
                Some(t) => t,
                None => return Ok(()), // at the root, or no pane: no-op
            };
            s.backends.require(&target)?;
            let focused = s.layout.focused();
            if let Some(pane) = s.layout.state_mut(focused) {
                pane.navigate(target.clone());
                expand_unless_tree_driven(pane);
            }
            s.note_visit(&target);
            s.ensure_read(target)?;
            Ok(())
        },
    }
}

/// `nav.back` — step back in the focused pane's history.
pub fn back() -> CommandSpec {
    CommandSpec {
        id: "nav.back",
        label: "Back",
        menu: &["Go", "Back"],
        shortcut: Some("Alt+Left"),
        params: "none",
        enabled: |s| focused_pane(s).is_some_and(|p| p.can_go_back()),
        run: |s, _p: &Params| {
            let focused = s.layout.focused();
            let moved = s.layout.state_mut(focused).is_some_and(|p| {
                let moved = p.go_back();
                expand_unless_tree_driven(p);
                moved
            });
            if moved && let Some(loc) = s.layout.state(focused).map(|p| p.location.clone()) {
                s.note_visit(&loc);
                s.ensure_read(loc)?;
            }
            Ok(())
        },
    }
}

/// `nav.forward` — step forward in the focused pane's history.
pub fn forward() -> CommandSpec {
    CommandSpec {
        id: "nav.forward",
        label: "Forward",
        menu: &["Go", "Forward"],
        shortcut: Some("Alt+Right"),
        params: "none",
        enabled: |s| focused_pane(s).is_some_and(|p| p.can_go_forward()),
        run: |s, _p: &Params| {
            let focused = s.layout.focused();
            let moved = s.layout.state_mut(focused).is_some_and(|p| {
                let moved = p.go_forward();
                expand_unless_tree_driven(p);
                moved
            });
            if moved && let Some(loc) = s.layout.state(focused).map(|p| p.location.clone()) {
                s.note_visit(&loc);
                s.ensure_read(loc)?;
            }
            Ok(())
        },
    }
}

/// `nav.refresh` — invalidate and re-read the focused pane's directory.
pub fn refresh() -> CommandSpec {
    CommandSpec {
        id: "nav.refresh",
        label: "Refresh",
        menu: &["Go", "Refresh"],
        shortcut: Some("Ctrl+R"),
        params: "none",
        enabled: |_s| true,
        run: |s, _p: &Params| {
            let focused = s.layout.focused();
            let loc = match s.layout.state(focused) {
                Some(p) => p.location.clone(),
                None => return Ok(()),
            };
            s.backends.require(&loc)?;
            s.refresh_read(loc)?;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open(), up(), back(), forward(), refresh()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Params;
    use fm_testkit::MemFs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn mem_session() -> (Session, MemFs, VfsUri) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/docs").expect("mkdirs");
        mem.mkdirs_str("/home/alice/pics").expect("mkdirs");
        mem.put_str("/home/alice/docs/a.txt", b"a").expect("put");
        s.backends.register(Arc::new(mem.clone()));
        let home = VfsUri::parse("/home/alice").expect("uri");
        s.layout.state_mut(1).expect("pane").location = home.clone();
        s.submit_read(home.clone()).expect("submit");
        (s, mem, home)
    }

    fn wait_ready(s: &mut Session, uri: &VfsUri) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs.state(uri).is_ready() {
                return;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn open_changes_location_and_loads_listing() {
        let (mut s, _mem, home) = mem_session();
        wait_ready(&mut s, &home);
        let docs = home.join("docs").expect("join");
        let params = Params::String(docs.to_string());
        (open().run)(&mut s, &params).expect("open");
        assert_eq!(s.layout.state(1).expect("pane").location, docs);
        wait_ready(&mut s, &docs);
        let (entries, _) = s.pane_view(1).expect("view");
        assert_eq!(entries.len(), 1, "a.txt");
    }

    #[test]
    fn open_expands_and_focuses_the_directory_in_the_tree() {
        let (mut s, _mem, home) = mem_session();
        let docs = home.join("docs").expect("uri");
        (open().run)(&mut s, &Params::String(docs.to_string())).expect("open");
        let pane = s.layout.state(s.layout.focused()).expect("pane");
        assert!(pane.tree.is_expanded(&docs));
        assert_eq!(pane.tree.focused.as_ref(), Some(&docs));
    }

    #[test]
    fn open_accepts_an_object_param_and_rejects_garbage() {
        let (mut s, _mem, _home) = mem_session();
        let mut map = serde_json::Map::new();
        map.insert("uri".to_string(), Params::String("/tmp".to_string()));
        (open().run)(&mut s, &Params::Object(map)).expect("open");
        assert_eq!(
            s.layout.state(1).expect("pane").location,
            VfsUri::parse("/tmp").expect("uri")
        );
        // Wrong types and junk strings are Err, never panic.
        assert!((open().run)(&mut s, &Params::Bool(true)).is_err());
        assert!((open().run)(&mut s, &Params::Number(3.into())).is_err());
        assert!((open().run)(&mut s, &Params::String("\u{0}:::".into())).is_err());
    }

    #[test]
    fn up_back_forward_walk_history() {
        let (mut s, _mem, home) = mem_session();
        wait_ready(&mut s, &home);
        let docs = home.join("docs").expect("join");
        let pics = home.join("pics").expect("join");
        (open().run)(&mut s, &Params::String(docs.to_string())).expect("open docs");
        (open().run)(&mut s, &Params::String(pics.to_string())).expect("open pics");
        assert!((back().enabled)(&s));
        (back().run)(&mut s, &Params::Null).expect("back");
        assert_eq!(s.layout.state(1).expect("pane").location, docs);
        assert!((forward().enabled)(&s));
        (forward().run)(&mut s, &Params::Null).expect("forward");
        assert_eq!(s.layout.state(1).expect("pane").location, pics);
        // Up from pics → home
        (up().run)(&mut s, &Params::Null).expect("up");
        assert_eq!(s.layout.state(1).expect("pane").location, home);
        // Back at the start of history: no-op, no error
        {
            let pane = s.layout.state_mut(1).expect("pane");
            while pane.go_back() {}
        }
        assert!(!(back().enabled)(&s));
        (back().run)(&mut s, &Params::Null).expect("noop back");
    }

    #[test]
    fn up_at_the_tree_root_is_a_disabled_noop() {
        let (mut s, _mem, home) = mem_session();
        // Root the pane at its location: up_target is None.
        s.layout.state_mut(1).expect("pane").tree = fm_layout::TreeState::new(home);
        assert!(!(up().enabled)(&s));
        (up().run)(&mut s, &Params::Null).expect("noop");
        assert_eq!(
            s.layout.state(1).expect("pane").location,
            VfsUri::parse("/home/alice").expect("uri")
        );
    }

    #[test]
    fn refresh_rereads_and_invalidates() {
        let (mut s, _mem, home) = mem_session();
        wait_ready(&mut s, &home);
        _mem.put_str("/home/alice/new.txt", b"n").expect("put");
        (refresh().run)(&mut s, &Params::Null).expect("refresh");
        wait_ready(&mut s, &home);
        let (entries, _) = s.pane_view(1).expect("view");
        assert_eq!(entries.len(), 3, "fresh listing includes new.txt");
    }

    #[test]
    fn open_without_a_backend_for_the_scheme_is_an_error() {
        let mut s = Session::default();
        let before = s.layout.state(1).expect("pane").location.clone();
        let err = (open().run)(&mut s, &Params::String("sftp://host/dir".to_string()));
        assert!(err.is_err());
        assert_eq!(
            s.layout.state(1).expect("pane").location,
            before,
            "location must not move without a backend"
        );
    }

    /// A refresh is a reload, not a jump: the pane keeps its selection, its
    /// focused row, its scroll offset and the tree's expanded/focused/scroll
    /// state; only the cached entries are replaced.
    #[test]
    fn refresh_preserves_selection_focus_and_tree_state() {
        let (mut s, _mem, home) = mem_session();
        wait_ready(&mut s, &home);
        let docs = home.join("docs").expect("join");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.select_only(Some("docs"), 1);
            pane.listing.scroll = 42.0;
            pane.tree.expand(docs.clone());
            pane.tree.focused = Some(docs.clone());
            pane.tree.scroll = 7.0;
        }
        (refresh().run)(&mut s, &Params::Null).expect("refresh");
        wait_ready(&mut s, &home);
        let pane = s.layout.state(1).expect("pane");
        assert!(pane.listing.is_selected("docs"), "selection survives");
        assert_eq!(pane.listing.focused_row, 1, "focused row survives");
        assert_eq!(pane.listing.scroll, 42.0, "scroll survives");
        assert!(pane.tree.is_expanded(&docs), "expansion survives");
        assert_eq!(
            pane.tree.focused.as_ref(),
            Some(&docs),
            "tree focus survives"
        );
        assert_eq!(pane.tree.scroll, 7.0, "tree scroll survives");
        assert_eq!(pane.location, home, "the location itself must not move");
    }

    /// Re-opening the current directory keeps the pane's place and does not
    /// push a duplicate history entry.
    #[test]
    fn reopening_the_current_directory_keeps_the_panes_place() {
        let (mut s, _mem, home) = mem_session();
        wait_ready(&mut s, &home);
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.listing.select_only(Some("docs"), 1);
            pane.listing.scroll = 42.0;
        }
        (open().run)(&mut s, &Params::String(home.to_string())).expect("reopen");
        let pane = s.layout.state(1).expect("pane");
        assert_eq!(pane.location, home);
        assert!(pane.listing.is_selected("docs"));
        assert_eq!(pane.listing.focused_row, 1);
        assert_eq!(pane.listing.scroll, 42.0);
        assert!(!pane.can_go_back(), "no duplicate history entry");
    }
}
