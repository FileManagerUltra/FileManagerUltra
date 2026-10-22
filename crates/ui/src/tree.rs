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
fn collect_rows(s: &Session, pane: PaneId, root: &VfsUri) -> Vec<Row> {
    let expanded = |uri: &VfsUri| {
        s.layout
            .state(pane)
            .is_some_and(|state| state.tree.is_expanded(uri))
    };
    let mut rows = vec![Row {
        uri: root.clone(),
        name: root.file_name().unwrap_or("/").to_string(),
        depth: 0,
        is_dir: true,
        expanded: true,
        state: RowState::Loading,
    }];
    let mut queue: Vec<(VfsUri, usize)> = vec![(root.clone(), 0)];
    while let Some((uri, depth)) = queue.pop() {
        if rows.len() >= MAX_ROWS {
            break;
        }
        match s.dirs.state(&uri) {
            DirState::Ready { entries, .. } => {
                mark(&mut rows, &uri, RowState::Ready);
                let mut dirs: Vec<(String, VfsUri)> = entries
                    .iter()
                    .filter(|e| e.kind == EntryKind::Dir)
                    .map(|e| (e.name.clone(), e.uri.clone()))
                    .collect();
                dirs.sort_by_key(|(name, _)| name.to_lowercase());
                for (name, child) in dirs.into_iter().rev() {
                    if rows.len() >= MAX_ROWS {
                        break;
                    }
                    let is_expanded = expanded(&child);
                    let state = match s.dirs.state(&child) {
                        DirState::Ready { .. } | DirState::Absent => RowState::Ready,
                        DirState::Loading { .. } => RowState::Loading,
                        DirState::Failed { .. } => RowState::Failed,
                    };
                    rows.push(Row {
                        uri: child.clone(),
                        name,
                        depth: depth + 1,
                        is_dir: true,
                        expanded: is_expanded,
                        state,
                    });
                    if is_expanded {
                        queue.push((child, depth + 1));
                    }
                }
            }
            DirState::Loading { .. } | DirState::Absent => {
                mark(&mut rows, &uri, RowState::Loading);
            }
            DirState::Failed { .. } => {
                mark(&mut rows, &uri, RowState::Failed);
            }
        }
    }
    // Deepest-first from the queue walk → reverse into pre-order.
    rows.reverse();
    rows
}

fn mark(rows: &mut [Row], uri: &VfsUri, state: RowState) {
    if let Some(row) = rows.iter_mut().find(|r| &r.uri == uri) {
        row.state = state;
    }
}

/// Keyboard handling for a focused row (`Right` expands, `Left` collapses,
/// `Enter` opens).
fn handle_key(row: &Row, key: Key) -> KeyOutcome {
    match key {
        Key::ArrowRight if row.is_dir && !row.expanded => KeyOutcome::Expand(row.uri.clone()),
        Key::ArrowLeft if row.is_dir && row.expanded => KeyOutcome::Collapse(row.uri.clone()),
        Key::ArrowLeft if row.depth > 0 => match row.uri.parent() {
            Some(parent) => KeyOutcome::Collapse(parent),
            None => KeyOutcome::None,
        },
        Key::Enter if row.is_dir => KeyOutcome::Open(row.uri.clone()),
        _ => KeyOutcome::None,
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
    let state = s.layout.state(pane)?;
    let root = state.tree.root.clone();
    let location = state.location.clone();
    let rows = collect_rows(s, pane, &root);

    let mut open: Option<VfsUri> = None;
    let mut expand: Vec<VfsUri> = Vec::new();
    let mut collapse: Vec<VfsUri> = Vec::new();
    let mut open_key: Option<VfsUri> = None;

    let max_chars = ((ui.available_width() / (tokens.font_size * 0.55)).max(8.0)) as usize;
    egui::ScrollArea::vertical()
        .id_salt(format!("fm-tree-{pane}"))
        .auto_shrink([false, true])
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
                    if response.clicked() {
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
                    if response.clicked() && row.is_dir {
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

    if let Some(state) = s.layout.state_mut(pane) {
        for uri in expand {
            state.tree.expand(uri);
        }
        for uri in collapse {
            state.tree.collapse(&uri);
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
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn session() -> (Session, MemFs) {
        let mut s = Session::default();
        let mem = MemFs::new();
        mem.mkdirs_str("/home/alice/docs").expect("mkdirs");
        mem.mkdirs_str("/home/alice/pics").expect("mkdirs");
        mem.put_str("/home/alice/docs/a.txt", b"a").expect("put");
        s.backends.register(Arc::new(mem.clone()));
        let home = VfsUri::parse("/home/alice").expect("uri");
        s.layout.state_mut(1).expect("pane").location = home.clone();
        s.submit_read(home).expect("submit");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            if s.dirs
                .state(&VfsUri::parse("/home/alice").expect("uri"))
                .is_ready()
            {
                break;
            }
            assert!(Instant::now() < deadline, "read never ready");
            std::thread::sleep(Duration::from_millis(5));
        }
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
}
