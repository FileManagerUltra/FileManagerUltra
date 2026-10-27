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
fn root_label(root: &VfsUri) -> String {
    if root.scheme() == fm_vfs::Scheme::Network {
        "Network".to_string()
    } else if root.scheme() == fm_vfs::Scheme::Servers {
        "Saved servers".to_string()
    } else if root.is_drive_list() {
        "This PC".to_string()
    } else {
        root.file_name().unwrap_or("/").to_string()
    }
}

fn collect_rows(s: &Session, pane: PaneId, root: &VfsUri) -> Vec<Row> {
    let expanded = |uri: &VfsUri| {
        s.layout
            .state(pane)
            .is_some_and(|state| state.tree.is_expanded(uri))
    };
    let mut rows: Vec<Row> = Vec::new();
    // Pending `(name, uri, depth)`. Children are pushed in reverse so the LIFO
    // pop visits siblings in sorted order, each one before its own children.
    let mut stack: Vec<(String, VfsUri, usize)> = vec![(root_label(root), root.clone(), 0)];
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
        let mut dirs: Vec<&fm_vfs::Entry> = entries
            .iter()
            .filter(|e| e.kind == EntryKind::Dir)
            .filter(|e| s.show_hidden || !fm_engine::is_hidden(e))
            .collect();
        dirs.sort_by_key(|e| e.name.to_lowercase());
        let dirs = dirs
            .into_iter()
            .map(|e| (e.display_name().to_string(), e.uri.clone()));
        for (name, child) in dirs.rev() {
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
        // Collapse the node (when open) and move the cursor to its parent.
        Key::ArrowLeft if row.depth == 0 => KeyOutcome::CollapseAll,
        Key::ArrowLeft => {
            let collapse = (row.is_dir && row.expanded).then(|| row.uri.clone());
            let focus = if row.depth > 0 {
                row.uri.parent()
            } else {
                None
            };
            if collapse.is_none() && focus.is_none() {
                KeyOutcome::None
            } else {
                KeyOutcome::Left { collapse, focus }
            }
        }
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
    Left {
        collapse: Option<VfsUri>,
        focus: Option<VfsUri>,
    },
    Open(VfsUri),
    /// `Left` on the root folds every directory below it.
    CollapseAll,
}

/// Render the tree for one pane. Returns the uri to open (`nav.open`), if the
/// user activated an entry.
pub fn show(ui: &mut Ui, s: &mut Session, pane: PaneId, tokens: &Tokens) -> Option<VfsUri> {
    let (root, location, mut remembered, saved_scroll, stale, reveal) = {
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
            state.tree.reveal,
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
    let tree_active = s.layout.state(pane).is_some_and(|p| p.tree.active);

    let mut open: Option<VfsUri> = None;
    let mut expand: Vec<VfsUri> = Vec::new();
    let mut collapse: Vec<VfsUri> = Vec::new();
    let mut open_key: Option<VfsUri> = None;
    let mut collapse_all = false;
    let mut move_focus_to: Option<VfsUri> = None;

    let mut revealed = false;
    let mut scroll_focus_done = false;
    let scroll_focus = s.layout.state(pane).is_some_and(|p| p.tree.scroll_focus);
    let saved_servers = rows
        .first()
        .is_some_and(|r| r.uri.scheme() == fm_vfs::Scheme::Servers);
    let max_chars = ((ui.available_width() / (tokens.font_size * 0.55)).max(8.0)) as usize;
    let output = egui::ScrollArea::both()
        .id_salt(format!("fm-tree-{pane}"))
        .auto_shrink([false, true])
        .vertical_scroll_offset(saved_scroll)
        .show(ui, |ui| {
            for row in &rows {
                let selected = row.uri == location;
                let indent = row.depth as f32 * (tokens.font_size + tokens.padding);
                ui.horizontal(|ui| {
                    ui.add_space(indent);
                    let label = match row.state {
                        RowState::Ready => {
                            ellipsize(&row.name, max_chars.saturating_sub(row.depth))
                        }
                        RowState::Loading => format!("{} …", ellipsize(&row.name, max_chars)),
                        RowState::Failed => format!("{} !", ellipsize(&row.name, max_chars)),
                    };
                    let response = ui.add(
                        egui::Button::selectable(selected, format!("   {label}"))
                            .sense(egui::Sense::click_and_drag()),
                    );
                    if reveal && selected {
                        response.scroll_to_me(Some(egui::Align::Center));
                        revealed = true;
                    }
                    if row.is_dir {
                        paint_chevron(ui, response.rect, row.expanded, selected, tokens);
                    }
                    if is_connected_host(s, &row.uri) {
                        // Beside the name, never over it.
                        let d = tokens.font_size * 0.6;
                        let (dot, _) =
                            ui.allocate_exact_size(egui::vec2(d, d), egui::Sense::hover());
                        ui.painter()
                            .circle_filled(dot.center(), d * 0.5, tokens.success);
                    }
                    if response.drag_started() {
                        response.dnd_set_drag_payload(crate::dnd::DragPayload {
                            sources: vec![row.uri.clone()],
                            origin: pane,
                        });
                    }
                    if row.is_dir {
                        crate::dnd::register_zone(
                            ui.ctx(),
                            response.rect,
                            row.uri.clone(),
                            crate::dnd::ZONE_ROW,
                        );
                    }
                    // egui turns Enter on a focused button into a click; Enter must only open.
                    let enter = response.has_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                    let clicked = response.clicked() && !enter;
                    if clicked {
                        response.request_focus();
                    }
                    if clicked && let Some(state) = s.layout.state_mut(pane) {
                        state.tree.active = true;
                    }
                    if response.has_focus()
                        && !tree_active
                        && let Some(state) = s.layout.state_mut(pane)
                    {
                        state.tree.active = true;
                    }
                    if clicked || response.has_focus() {
                        // Focus moved to this node — remember it (pane state is
                        // what survives the rows disappearing during a reload).
                        if remembered.as_ref() != Some(&row.uri) {
                            if !clicked {
                                // Keyboard moved focus here: keep the row on screen.
                                response.scroll_to_me(None);
                            }
                            remembered = Some(row.uri.clone());
                            if let Some(state) = s.layout.state_mut(pane) {
                                state.tree.focused = remembered.clone();
                            }
                        }
                    } else if tree_active
                        && egui_focused.is_none()
                        && remembered.as_ref() == Some(&row.uri)
                    {
                        // Focus was dropped while this row was gone: take it back.
                        response.request_focus();
                        if scroll_focus {
                            response.scroll_to_me(None);
                            scroll_focus_done = true;
                        }
                    }
                    response
                        .clone()
                        .on_hover_text(fm_engine::item_cmds::display_path(&row.uri));
                    if response.has_focus() {
                        // Left/Right belong to expand/collapse; without the lock egui moves
                        // widget focus sideways (into the listing) before we see the key.
                        ui.memory_mut(|m| {
                            m.set_focus_lock_filter(
                                response.id,
                                egui::EventFilter {
                                    horizontal_arrows: true,
                                    ..Default::default()
                                },
                            );
                        });
                        ui.input(|i| {
                            for key in [Key::ArrowRight, Key::ArrowLeft, Key::Enter] {
                                if !i.key_pressed(key) {
                                    continue;
                                }
                                match handle_key(row, key) {
                                    KeyOutcome::None => {}
                                    KeyOutcome::Expand(uri) => expand.push(uri),
                                    KeyOutcome::Left { collapse: c, focus } => {
                                        collapse.extend(c);
                                        if focus.is_some() {
                                            move_focus_to = focus;
                                        }
                                    }
                                    KeyOutcome::Open(uri) => open_key = Some(uri),
                                    KeyOutcome::CollapseAll => collapse_all = true,
                                }
                            }
                        });
                    }
                    if move_focus_to.is_some() && response.has_focus() {
                        // Dropping egui focus makes the "focus was dropped" branch hand it to
                        // `tree.focused` (the parent) on the next frame.
                        ui.memory_mut(|m| m.surrender_focus(response.id));
                    }
                    if clicked && row.is_dir {
                        // Selecting a folder shows it in the listing.
                        open = Some(row.uri.clone());
                        if row.expanded {
                            collapse.push(row.uri.clone());
                        } else {
                            expand.push(row.uri.clone());
                        }
                    }
                    if response.double_clicked() {
                        open = Some(row.uri.clone());
                    }
                    if saved_servers && row.depth == 1 {
                        crate::context_menu::attach_server(
                            &response, s, &row.name, &row.uri, &mut open,
                        );
                    }
                });
            }
        });

    // Persist the scroll offset — but never the value egui clamped while the
    // rows were missing: a refresh shrinks the tree to the root for a few
    // frames, and saving that would pin the tree to the top afterwards.
    // A failed node (permission denied, gone) is settled and keeps its place, so it must not
    // block persisting: otherwise the saved offset is forced back every frame.
    // A reveal just scrolled the node into view: keep that offset even while other rows load.
    if (revealed || rows.iter().all(|r| r.state != RowState::Loading))
        && let Some(state) = s.layout.state_mut(pane)
    {
        state.tree.scroll = output.state.clamped_offset().y;
    }

    if (revealed || rows.iter().all(|r| r.state == RowState::Ready))
        && let Some(state) = s.layout.state_mut(pane)
    {
        state.tree.reveal = false;
    }
    if scroll_focus_done && let Some(state) = s.layout.state_mut(pane) {
        state.tree.scroll_focus = false;
    }
    if let Some(parent) = move_focus_to
        && let Some(state) = s.layout.state_mut(pane)
    {
        state.tree.focused = Some(parent);
        state.tree.scroll_focus = true;
    }
    for uri in expand {
        expand_node(s, pane, &uri);
    }
    if let Some(state) = s.layout.state_mut(pane).filter(|_| collapse_all) {
        state.tree.collapse_all();
    }
    for uri in &collapse {
        if let Some(state) = s.layout.state_mut(pane) {
            state.tree.collapse(uri);
            // The focused row is about to vanish with its parent: keep focus on the parent
            // so Right can expand it again.
            if state
                .tree
                .focused
                .as_ref()
                .is_some_and(|f| uri.is_ancestor_of(f))
            {
                state.tree.focused = Some(uri.clone());
            }
        }
    }
    open.or(open_key)
}

/// Disclosure triangle (right when collapsed, down when expanded) painted at the row's left.
/// A server row (`smb://host/`, `sftp://host/`) with a live connection: shown with a green dot.
fn is_connected_host(s: &Session, uri: &VfsUri) -> bool {
    matches!(uri.scheme(), fm_vfs::Scheme::Smb | fm_vfs::Scheme::Sftp)
        && uri.path_str() == "/"
        && s.remote.state(uri.scheme(), uri.authority())
            == Some(&fm_engine::remote::ConnState::Ready)
}

fn paint_chevron(ui: &Ui, rect: egui::Rect, expanded: bool, selected: bool, tokens: &Tokens) {
    let size = tokens.font_size * 0.35;
    let c = egui::pos2(rect.left() + tokens.padding + size, rect.center().y);
    let pts = if expanded {
        vec![
            c + egui::vec2(-size, -size * 0.5),
            c + egui::vec2(size, -size * 0.5),
            c + egui::vec2(0.0, size * 0.7),
        ]
    } else {
        vec![
            c + egui::vec2(-size * 0.5, -size),
            c + egui::vec2(-size * 0.5, size),
            c + egui::vec2(size * 0.7, 0.0),
        ]
    };
    let color = if selected {
        tokens.selection_text
    } else {
        tokens.text_muted
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
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
            KeyOutcome::Left {
                collapse: Some(uri.clone()),
                focus: uri.parent()
            }
        );
        assert_eq!(handle_key(&row, Key::Enter), KeyOutcome::Open(uri));
        let mut root = row.clone();
        root.depth = 0;
        assert_eq!(handle_key(&root, Key::ArrowLeft), KeyOutcome::CollapseAll);
        // Left on a collapsed child only moves the cursor to its parent.
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
            KeyOutcome::Left {
                collapse: None,
                focus: VfsUri::parse("/home/alice/docs").ok()
            }
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
            pane.tree.active = true;
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

    /// Navigating to a deep folder scrolls the tree to it, even while rows are still loading.
    #[test]
    fn navigating_scrolls_the_tree_to_the_folder() {
        let mut s = Session::default();
        let mem = MemFs::new();
        for i in 0..60 {
            mem.mkdirs_str(&format!("/d{i:02}")).expect("mkdirs");
        }
        for i in 0..30 {
            mem.mkdirs_str(&format!("/zz/e{i:02}")).expect("mkdirs");
        }
        mem.mkdirs_str("/zz/target").expect("mkdirs");
        s.backends.register(Arc::new(mem));
        let root = VfsUri::parse("/").expect("uri");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.tree = fm_layout::TreeState::new(root.clone());
            pane.location = root.clone();
        }
        s.submit_read(root.clone()).expect("submit");
        wait_ready(&mut s, &root);
        let ctx = egui::Context::default();
        run_tree_frame(&ctx, &mut s, 1);

        let target = VfsUri::parse("/zz/target").expect("uri");
        s.layout
            .state_mut(1)
            .expect("pane")
            .navigate(target.clone());
        for _ in 0..40 {
            s.poll();
            run_tree_frame(&ctx, &mut s, 1);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let scroll = s.layout.state(1).expect("pane").tree.scroll;
        assert!(scroll > 100.0, "tree scrolled to the folder, got {scroll}");
    }

    /// `Right` expands a node; it must not hand egui focus to a widget beside the tree.
    #[test]
    fn right_arrow_keeps_focus_in_the_tree() {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/a").expect("mkdirs");
        mem.mkdirs_str("/home/alice/b").expect("mkdirs");
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
        let tokens = Tokens::dark();
        let beside = egui::Id::NULL.with("beside-the-tree");
        let frame = |ctx: &egui::Context, s: &mut Session, keys: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events: keys,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.horizontal_top(|ui| {
                            ui.allocate_ui(egui::vec2(200.0, 500.0), |ui| {
                                let _ = show(ui, s, 1, &tokens);
                            });
                            let rect = ui.allocate_space(egui::vec2(100.0, 20.0)).1;
                            ui.interact(rect, beside, egui::Sense::focusable_noninteractive());
                        });
                    });
                },
            );
            out.textures_delta.clear();
        };
        frame(&ctx, &mut s, vec![]);
        let a = home.join("a").expect("join");
        let pane = s.layout.state_mut(1).expect("pane");
        pane.tree.focused = Some(a);
        pane.tree.active = true;
        frame(&ctx, &mut s, vec![]);
        frame(&ctx, &mut s, vec![]);
        assert!(ctx.memory(|m| m.focused()).is_some());
        for _ in 0..2 {
            frame(
                &ctx,
                &mut s,
                vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        assert_ne!(ctx.memory(|m| m.focused()), Some(beside));
    }

    #[test]
    fn left_collapses_and_moves_the_cursor_to_the_parent() {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/a/x").expect("mkdirs");
        mem.mkdirs_str("/home/alice/b").expect("mkdirs");
        s.backends.register(Arc::new(mem));
        let home = VfsUri::parse("/home/alice").expect("uri");
        let a = home.join("a").expect("join");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.tree = fm_layout::TreeState::new(home.clone());
            pane.location = home.clone();
            pane.tree.focused = Some(a.clone());
            pane.tree.active = true;
            pane.tree.expand(a.clone());
        }
        s.submit_read(home.clone()).expect("submit");
        wait_ready(&mut s, &home);
        s.submit_read(a.clone()).expect("submit");
        wait_ready(&mut s, &a);
        let ctx = egui::Context::default();
        let key = |k| {
            vec![egui::Event::Key {
                key: k,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        let tokens = Tokens::dark();
        let frame = |s: &mut Session, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = show(ui, s, 1, &tokens);
                    });
                },
            );
            out.textures_delta.clear();
        };
        for _ in 0..3 {
            frame(&mut s, vec![]);
        }
        frame(&mut s, key(egui::Key::ArrowLeft));
        for _ in 0..3 {
            frame(&mut s, vec![]);
        }
        let pane = s.layout.state(1).expect("pane");
        assert!(!pane.tree.is_expanded(&a), "collapsed");
        assert_eq!(pane.tree.focused.as_ref(), Some(&home), "cursor on parent");
    }

    #[test]
    fn left_on_a_leaf_moves_the_cursor_to_its_parent_without_collapsing_it() {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/a/x").expect("mkdirs");
        s.backends.register(Arc::new(mem));
        let home = VfsUri::parse("/home/alice").expect("uri");
        let a = home.join("a").expect("join");
        let x = a.join("x").expect("join");
        {
            let pane = s.layout.state_mut(1).expect("pane");
            pane.tree = fm_layout::TreeState::new(home.clone());
            pane.location = home.clone();
            pane.tree.focused = Some(x.clone());
            pane.tree.active = true;
            pane.tree.expand(a.clone());
        }
        for u in [&home, &a] {
            s.submit_read(u.clone()).expect("submit");
            wait_ready(&mut s, u);
        }
        let ctx = egui::Context::default();
        let tokens = Tokens::dark();
        let frame = |s: &mut Session, k: Option<egui::Key>| {
            let events = k
                .map(|key| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .into_iter()
                .collect();
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = show(ui, s, 1, &tokens);
                    });
                },
            );
            out.textures_delta.clear();
        };
        for _ in 0..3 {
            frame(&mut s, None);
        }
        frame(&mut s, Some(egui::Key::ArrowLeft));
        for _ in 0..3 {
            frame(&mut s, None);
        }
        let pane = s.layout.state(1).expect("pane");
        assert!(pane.tree.is_expanded(&a), "parent stays open");
        assert_eq!(pane.tree.focused.as_ref(), Some(&a));
    }
}
