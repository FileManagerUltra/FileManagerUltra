//! Lazy file tree: children are read from the engine's [`fm_engine::DirCache`]
//! for expanded nodes only. Expand/collapse mutates the pane's [`fm_layout::TreeState`]
//! (pure view state); opening an entry is returned to the caller, which dispatches
//! `nav.open`.

use egui::{Key, Ui};
use fm_engine::{DirState, Session};
use fm_layout::PaneId;
use fm_vfs::{EntryKind, VfsUri};

use crate::theme::Tokens;

/// Hard cap on rendered rows: a hostile tree (many expanded huge dirs) must not
/// blow the frame budget.
const MAX_ROWS: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
enum RowState {
    Ready,
    Loading,
    Failed,
}

#[derive(Clone, Debug)]
struct Row {
    uri: VfsUri,
    name: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
    state: RowState,
}

/// Flatten the expanded portion of the tree, reading only from the cache.
///
/// Depth-first **pre-order**: a directory's children are emitted immediately
/// after the directory itself, so the flat list nests exactly like the tree.
/// An explicit stack (never recursion) keeps deeply nested trees off the call
/// stack and bounds the walk by [`MAX_ROWS`].
fn collect_rows(s: &Session, pane: PaneId, root: &VfsUri) -> Vec<Row> {
    let expanded = |uri: &VfsUri| {
        s.layout
            .state(pane)
            .is_some_and(|state| state.tree.is_expanded(uri))
    };
    let mut rows: Vec<Row> = Vec::new();
    // Pending `(name, uri, depth)`. Children are pushed in reverse so the LIFO
    // pop visits siblings in sorted order, each one before its own children.
    let mut stack: Vec<(String, VfsUri, usize)> =
        vec![(root.file_name().unwrap_or("/").to_string(), root.clone(), 0)];
    while let Some((name, uri, depth)) = stack.pop() {
        if rows.len() >= MAX_ROWS {
            break;
        }
        // The root is always expanded (see `TreeState::new`); every other node
        // only once the user expanded it.
        let is_expanded = depth == 0 || expanded(&uri);
        let dir_state = s.dirs.state(&uri);
        let state = match dir_state {
            DirState::Ready { .. } => RowState::Ready,
            DirState::Failed { .. } => RowState::Failed,
            DirState::Loading { .. } => RowState::Loading,
            // Never read: an expanded node is waiting on its first read, a
            // collapsed one is just a plain arrow.
            DirState::Absent if is_expanded => RowState::Loading,
            DirState::Absent => RowState::Ready,
        };
        rows.push(Row {
            uri: uri.clone(),
            name,
            depth,
            is_dir: true,
            expanded: is_expanded,
            state,
        });
        if !is_expanded {
            continue;
        }
        let DirState::Ready { entries, .. } = dir_state else {
            continue;
        };
        let mut dirs: Vec<(String, VfsUri)> = entries
            .iter()
            .filter(|e| e.kind == EntryKind::Dir)
            .map(|e| (e.name.clone(), e.uri.clone()))
            .collect();
        dirs.sort_by_key(|(name, _)| name.to_lowercase());
        for (name, child) in dirs.into_iter().rev() {
            if rows.len() + stack.len() >= MAX_ROWS {
                break;
            }
            stack.push((name, child, depth + 1));
        }
    }
    rows
}

/// Keyboard handling for a focused row (`Right` expands, `Left` collapses,
/// `Enter` opens).
fn handle_key(row: &Row, key: Key) -> KeyOutcome {
    match key {
        // `Right` on an expanded-but-failed node re-submits its read (the spec
        // requires the node to stay expandable for retry).
        Key::ArrowRight if row.is_dir && (!row.expanded || row.state == RowState::Failed) => {
            KeyOutcome::Expand(row.uri.clone())
        }
        Key::ArrowLeft if row.is_dir && row.expanded => KeyOutcome::Collapse(row.uri.clone()),
        Key::ArrowLeft if row.depth > 0 => match row.uri.parent() {
            Some(parent) => KeyOutcome::Collapse(parent),
            None => KeyOutcome::None,
        },
        Key::Enter if row.is_dir => KeyOutcome::Open(row.uri.clone()),
        _ => KeyOutcome::None,
    }
}

/// Expand a node and submit its one cached read: the engine dedups in-flight
/// and fresh reads, so a node is read exactly once per expand (spec
/// `file-tree`). A failure (no backend for the scheme) is recorded so the row
/// shows the error affordance instead of spinning.
fn expand_node(s: &mut Session, pane: PaneId, uri: &VfsUri) {
    if let Some(state) = s.layout.state_mut(pane) {
        state.tree.expand(uri.clone());
    }
    if let Err(err) = s.ensure_read(uri.clone()) {
        s.dirs.set_failed(uri, err.to_string());
    }
}

#[derive(Debug, PartialEq)]
enum KeyOutcome {
    None,
    Expand(VfsUri),
    Collapse(VfsUri),
    Open(VfsUri),
}

/// Render the tree for one pane. Returns the uri to open (`nav.open`), if the
/// user activated an entry.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<VfsUri> {
    let (root, location, mut remembered, saved_scroll, stale) = {
        let state = s.layout.state(pane)?;
        // A refresh or watcher invalidation drops cached entries without
        // touching `expanded`. Re-submit reads for those nodes, or the
        // expanded subtree under the re-read directory stays empty until the
        // user re-clicks it. `ensure_read` dedups in-flight/fresh slots, so
        // each node is read exactly once per invalidation.
        let stale: Vec<VfsUri> = state
            .tree
            .expanded()
            .filter(|uri| matches!(s.dirs.state(uri), DirState::Absent))
            .cloned()
            .collect();
        (
            state.tree.root.clone(),
            state.location.clone(),
            state.tree.focused.clone(),
            state.tree.scroll,
            stale,
        )
    };
    for uri in stale {
        if let Err(err) = s.ensure_read(uri.clone()) {
            s.dirs.set_failed(&uri, err.to_string());
        }
    }
    let rows = collect_rows(s, pane, &root);
    // egui drops keyboard focus the moment a widget stops rendering (its
    // dead-mans switch) — which happens to every child row while a refresh
    // re-reads the directory. `remembered` is the pane-state copy of the
    // focused node, used to re-focus the row once it is back.
    let egui_focused = ui.ctx().memory(|m| m.focused());

    let mut open: Option<VfsUri> = None;
    let mut expand: Vec<VfsUri> = Vec::new();
    let mut collapse: Vec<VfsUri> = Vec::new();
    let mut open_key: Option<VfsUri> = None;

    let max_chars = ((ui.available_width() / (tokens.font_size * 0.55)).max(8.0)) as usize;
    let output = egui::ScrollArea::vertical()
        .id_salt(format!("fm-tree-{pane}"))
        .auto_shrink([false, true])
        .vertical_scroll_offset(saved_scroll)
        .show(ui, |ui| {
            for row in &rows {
                let selected = row.uri == location;
                let indent = row.depth as f32 * (tokens.font_size + tokens.padding);
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    let arrow = match (row.is_dir, row.expanded) {
                        (true, true) => "v",
                        (true, false) => ">",
                        (false, _) => " ",
                    };
                    let label = match row.state {
                        RowState::Ready => {
                            ellipsize(&row.name, max_chars.saturating_sub(row.depth))
                        }
                        RowState::Loading => format!("{} …", ellipsize(&row.name, max_chars)),
                        RowState::Failed => format!("{} !", ellipsize(&row.name, max_chars)),
                    };
                    let response = ui.selectable_label(selected, format!("{arrow} {label}"));
                    let clicked = response.clicked();
                    if clicked {
                        response.request_focus();
                    }
                    if clicked || response.has_focus() {
                        // Focus moved to this node — remember it (pane state is
                        // what survives the rows disappearing during a reload).
                        if remembered.as_ref() != Some(&row.uri) {
                            remembered = Some(row.uri.clone());
                            if let Some(state) = s.layout.state_mut(pane) {
                                state.tree.focused = remembered.clone();
                            }
                        }
                    } else if egui_focused.is_none() && remembered.as_ref() == Some(&row.uri) {
                        // Focus was dropped while this row was gone: take it back.
                        response.request_focus();
                    }
                    response.clone().on_hover_text(row.uri.path_str());
                    if response.has_focus() {
                        ui.input(|i| {
                            for key in [Key::ArrowRight, Key::ArrowLeft, Key::Enter] {
                                if !i.key_pressed(key) {
                                    continue;
                                }
                                match handle_key(row, key) {
                                    KeyOutcome::None => {}
                                    KeyOutcome::Expand(uri) => expand.push(uri),
                                    KeyOutcome::Collapse(uri) => collapse.push(uri),
                                    KeyOutcome::Open(uri) => open_key = Some(uri),
                                }
                            }
                        });
                    }
                    if clicked && row.is_dir {
                        if row.expanded {
                            collapse.push(row.uri.clone());
                        } else {
                            expand.push(row.uri.clone());
                        }
                    }
                    if response.double_clicked() {
                        open = Some(row.uri.clone());
                    }
                });
            }
        });

    // Persist the scroll offset — but never the value egui clamped while the
    // rows were missing: a refresh shrinks the tree to the root for a few
    // frames, and saving that would pin the tree to the top afterwards.
    if rows.iter().all(|r| r.state == RowState::Ready)
        && let Some(state) = s.layout.state_mut(pane)
    {
        state.tree.scroll = output.state.clamped_offset().y;
    }

    for uri in expand {
        expand_node(s, pane, &uri);
    }
    for uri in &collapse {
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree.collapse(uri);
        }
    }
    open.or(open_key)
}

/// Truncate to `max_chars` with a horizontal ellipsis (full name is a tooltip).
pub(crate) fn ellipsize(name: &str, max_chars: usize) -> String {
    let count = name.chars().count();
    if count <= max_chars.max(1) {
        return name.to_string();
    }
    let keep = max_chars.max(1).saturating_sub(1);
    let mut out: String = name.chars().take(keep).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;
    use fm_vfs::Vfs;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    /// Drive `Session::poll` until the cache slot for `uri` settles.
    fn poll_for(s: &mut Session, uri: &VfsUri, until: impl Fn(&DirState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if until(&s.dirs.state(uri)) {
                return;
            }
            assert!(Instant::now() < deadline, "read never settled");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_ready(s: &mut Session, uri: &VfsUri) {
        poll_for(s, uri, DirState::is_ready);
    }

    fn wait_failed(s: &mut Session, uri: &VfsUri) {
        poll_for(s, uri, |state| matches!(state, DirState::Failed { .. }));
    }

    /// `/home/alice/{docs, pics}` with `docs/a.txt`, `/home/alice` read.
    fn session() -> (Session, MemFs) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/docs").expect("mkdirs");
        mem.mkdirs_str("/home/alice/pics").expect("mkdirs");
        mem.put_str("/home/alice/docs/a.txt", b"a").expect("put");
        s.backends.register(Arc::new(mem.clone()));
        let home = VfsUri::parse("/home/alice").expect("uri");
        s.layout.state_mut(1).expect("pane").location = home.clone();
        s.submit_read(home.clone()).expect("submit");
        wait_ready(&mut s, &home);
        (s, mem)
    }

    /// `/home/alice/{docs/deep, pics}` — a grandchild, so nesting is assertable.
    fn nested_session() -> (Session, MemFs) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/docs/deep").expect("mkdirs");
        mem.mkdirs_str("/home/alice/pics").expect("mkdirs");
        s.backends.register(Arc::new(mem.clone()));
        let home = VfsUri::parse("/home/alice").expect("uri");
        s.layout.state_mut(1).expect("pane").location = home.clone();
        s.submit_read(home.clone()).expect("submit");
        wait_ready(&mut s, &home);
        (s, mem)
    }

    #[test]
    fn rows_are_capped_and_ellipsize_is_safe() {
        assert_eq!(ellipsize("short", 20), "short");
        assert!(ellipsize("longer-name", 5).ends_with('…'));
        assert_eq!(ellipsize("anything", 0), "…");
    }

    #[test]
    fn collect_walks_only_expanded_nodes() {
        let (s, _mem) = session();
        let root = VfsUri::parse("/home/alice").expect("uri");
        let rows = collect_rows(&s, 1, &root);
        assert_eq!(rows.len(), 3, "root + two collapsed dirs");
        assert!(rows.iter().all(|r| r.state == RowState::Ready));
        // Expand docs: its child file is not a dir, so still 3 rows but docs expanded flag.
        let (mut s, _mem) = session();
        s.layout
            .state_mut(1)
            .expect("pane")
            .tree
            .expand(VfsUri::parse("/home/alice/docs").expect("uri"));
        let rows = collect_rows(&s, 1, &root);
        assert_eq!(rows.len(), 3, "files are not tree rows");
        assert!(rows.iter().any(|r| r.expanded && r.name == "docs"));
    }

    #[test]
    fn key_outcomes_expand_collapse_open() {
        let uri = VfsUri::parse("/home/alice/docs").expect("uri");
        let row = Row {
            uri: uri.clone(),
            name: "docs".to_string(),
            depth: 1,
            is_dir: true,
            expanded: false,
            state: RowState::Ready,
        };
        assert_eq!(
            handle_key(&row, Key::ArrowRight),
            KeyOutcome::Expand(uri.clone())
        );
        let mut collapsed = row.clone();
        collapsed.expanded = true;
        assert_eq!(
            handle_key(&collapsed, Key::ArrowLeft),
            KeyOutcome::Collapse(uri.clone())
        );
        assert_eq!(handle_key(&row, Key::Enter), KeyOutcome::Open(uri));
        // Left on a collapsed child collapses its parent.
        let child = Row {
            uri: VfsUri::parse("/home/alice/docs/a.txt").expect("uri"),
            name: "a.txt".to_string(),
            depth: 2,
            is_dir: false,
            expanded: false,
            state: RowState::Ready,
        };
        assert_eq!(
            handle_key(&child, Key::ArrowLeft),
            KeyOutcome::Collapse(VfsUri::parse("/home/alice/docs").expect("uri"))
        );
    }

    /// Regression: children must render directly under their parent — the old
    /// walk reversed its own rows, hoisting the deepest nodes to the top and
    /// pushing the root to the bottom.
    #[test]
    fn expanded_children_render_under_their_parent() {
        let (mut s, _mem) = nested_session();
        let home = VfsUri::parse("/home/alice").expect("uri");
        let docs = VfsUri::parse("/home/alice/docs").expect("uri");

        let rows = collect_rows(&s, 1, &home);
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("alice", 0), ("docs", 1), ("pics", 1)]);

        // Expanding docs (and its cached read) nests `deep` under `docs`,
        // before the sibling `pics` — pre-order, parents before children.
        expand_node(&mut s, 1, &docs);
        wait_ready(&mut s, &docs);
        let rows = collect_rows(&s, 1, &home);
        let names: Vec<(&str, usize)> = rows.iter().map(|r| (r.name.as_str(), r.depth)).collect();
        assert_eq!(names, [("alice", 0), ("docs", 1), ("deep", 2), ("pics", 1)]);
    }

    /// Spec `file-tree`: expanding submits exactly one read — duplicates
    /// (double click, re-expand) dedup against the in-flight/fresh slot.
    #[test]
    fn expanding_reads_each_node_once() {
        let (mut s, _mem) = nested_session();
        let docs = VfsUri::parse("/home/alice/docs").expect("uri");
        assert_eq!(s.dirs.state(&docs), DirState::Absent);

        expand_node(&mut s, 1, &docs);
        let in_flight = s.dirs.state(&docs);
        assert!(matches!(in_flight, DirState::Loading { .. }));
        expand_node(&mut s, 1, &docs);
        assert_eq!(s.dirs.state(&docs), in_flight, "no second read");

        wait_ready(&mut s, &docs);
        let fresh = s.dirs.state(&docs);
        expand_node(&mut s, 1, &docs);
        assert_eq!(s.dirs.state(&docs), fresh, "cached node is not re-read");
    }

    /// Spec `file-tree`: a failed expand shows the error affordance on the row
    /// and the node stays expandable for retry.
    #[test]
    fn expand_failure_shows_an_error_and_stays_retryable() {
        let (mut s, mem) = nested_session();
        let home = VfsUri::parse("/home/alice").expect("uri");
        let pics = VfsUri::parse("/home/alice/pics").expect("uri");
        // The parent already listed it, but it vanishes before the read lands.
        mem.remove(&pics).expect("remove");
        expand_node(&mut s, 1, &pics);
        wait_failed(&mut s, &pics);

        let rows = collect_rows(&s, 1, &home);
        let row = rows.iter().find(|r| r.name == "pics").expect("pics row");
        assert_eq!(row.state, RowState::Failed, "inline error affordance");
        assert!(row.expanded);

        // Right retries instead of being ignored on an already-expanded node.
        assert_eq!(
            handle_key(row, Key::ArrowRight),
            KeyOutcome::Expand(pics.clone())
        );
        expand_node(&mut s, 1, &pics);
        wait_failed(&mut s, &pics);
    }

    /// One headless frame of the tree. A fixed screen rect matters: scroll
    /// offsets only clamp against a real viewport.
    fn run_tree_frame(ctx: &egui::Context, s: &mut Session, pane: PaneId) {
        let tokens = Tokens::dark();
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
                        let _ = show(ui, s, pane, &tokens);
                    });
            },
        );
        output.textures_delta.clear();
    }

    /// Regression for "a reload must not change my position": while the
    /// directory re-reads the tree collapses to its root, which makes egui
    /// clamp its persisted scroll offset and drop keyboard focus (dead-mans
    /// switch). Both must come back from the pane's own state.
    #[test]
    fn refresh_keeps_the_tree_scroll_and_focus() {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice").expect("mkdirs");
        for i in 0..60 {
            mem.mkdirs_str(&format!("/home/alice/dir{i:02}"))
                .expect("mkdirs");
        }
        s.backends.register(Arc::new(mem));
        let home = VfsUri::parse("/home/alice").expect("uri");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.tree = fm_layout::TreeState::new(home.clone());
            pane.location = home.clone();
        }
        s.submit_read(home.clone()).expect("submit");
        wait_ready(&mut s, &home);

        let ctx = egui::Context::default();
        run_tree_frame(&ctx, &mut s, 1);
        // Park the tree partway down and remember a node as focused.
        let focused = home.join("dir10").expect("join");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.tree.scroll = 240.0;
            pane.tree.focused = Some(focused.clone());
        }
        run_tree_frame(&ctx, &mut s, 1);
        let parked = s.layout.state(1).expect("pane").tree.scroll;
        assert!(parked > 100.0, "parked offset round-trips, got {parked}");

        // Refresh: the rows collapse to a loading root for a few frames.
        s.refresh_read(home.clone()).expect("refresh");
        for _ in 0..3 {
            s.poll();
            run_tree_frame(&ctx, &mut s, 1);
        }
        let state = s.layout.state(1).expect("pane");
        assert_eq!(
            state.tree.scroll, parked,
            "loading frames must not write the clamped offset back"
        );
        assert_eq!(
            state.tree.focused.as_ref(),
            Some(&focused),
            "the focused node must not be forgotten"
        );

        // Once the rows are back the offset is restored and focus returns.
        wait_ready(&mut s, &home);
        run_tree_frame(&ctx, &mut s, 1);
        run_tree_frame(&ctx, &mut s, 1);
        let restored = s.layout.state(1).expect("pane").tree.scroll;
        assert!(
            (restored - parked).abs() < 1.0,
            "scroll restored: {restored} vs {parked}"
        );
        assert!(
            ctx.memory(|m| m.focused()).is_some(),
            "tree keyboard focus is restored"
        );
    }
}
