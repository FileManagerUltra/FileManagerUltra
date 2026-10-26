//! Trash manager commands: list, restore, purge and empty the platform trash.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub use fm_ops::{TrashChild, TrashChildren, TrashEntry, TrashListing};

use fm_layout::PaneKind;
use fm_ops::{
    empty_trash, list_trash, list_trash_dir, purge_trash, restore_trash, restore_trash_child,
};
use fm_vfs::FmError;

use crate::file_cmds::object;
use crate::jobs::JobLane;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

const MAX_IDS: usize = 100_000;
const MAX_ID_LEN: usize = 4096;
const MAX_DEPTH: usize = 64;
const SEP: char = '\u{0}';
const VIEW_KEY: &str = "\u{0}view";

enum Outcome {
    Listed(Result<TrashListing, String>),
    Acted(Result<String, String>),
}

type Slot = Arc<Mutex<Option<Outcome>>>;
type ChildSlot = Arc<Mutex<Vec<(String, Result<TrashChildren, String>)>>>;

/// A trashed folder's contents, as far as they have been read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Children {
    Loading,
    Ready(TrashChildren),
    Failed(String),
}

/// The folder the pane has opened: a trashed entry and a path inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashView {
    pub id: String,
    pub name: String,
    pub path: Vec<String>,
    pub children: Children,
}

/// Key of an expanded folder in [`TrashState::expanded`].
pub fn child_key(id: &str, path: &[String]) -> String {
    let mut key = id.to_string();
    for part in path {
        key.push(SEP);
        key.push_str(part);
    }
    key
}

/// State of the trash manager panel (capability `trash-and-undo`).
#[derive(Debug, Default)]
pub struct TrashState {
    pub busy: bool,
    /// A listing (or its failure) has arrived at least once.
    pub loaded: bool,
    pub listing: TrashListing,
    /// Ids selected in the panel.
    pub selected: HashSet<String>,
    /// Items picked from inside trashed folders: (entry id, path inside it).
    pub selected_children: HashSet<(String, Vec<String>)>,
    pub error: Option<String>,
    pub notice: Option<String>,
    /// Folders expanded inline, keyed by [`child_key`].
    pub expanded: HashMap<String, Children>,
    /// Set while the pane shows the inside of a trashed folder.
    pub view: Option<TrashView>,
    slot: SlotCell,
    child_slot: ChildSlotCell,
}

#[derive(Default)]
struct SlotCell(Slot);

impl std::fmt::Debug for SlotCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Slot")
    }
}

#[derive(Default)]
struct ChildSlotCell(ChildSlot);

impl std::fmt::Debug for ChildSlotCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChildSlot")
    }
}

/// `id` and `path` params shared by the browsing commands.
fn target(params: &Params) -> Result<(String, Vec<String>), FmError> {
    let bad = |m: &str| FmError::InvalidParams(m.to_string());
    let map = object(params)?.ok_or_else(|| bad("id is required"))?;
    let id = match map.get("id").and_then(Params::as_str) {
        Some(s) if !s.is_empty() && s.len() <= MAX_ID_LEN => s.to_string(),
        _ => return Err(bad("id must be a string of 1-4096 bytes")),
    };
    let path = match map.get("path") {
        None => Vec::new(),
        Some(v) => {
            let arr = v.as_array().ok_or_else(|| bad("path must be an array"))?;
            if arr.len() > MAX_DEPTH {
                return Err(bad("path is too deep"));
            }
            arr.iter()
                .map(|p| match p.as_str() {
                    Some(s) if !s.is_empty() && s.len() <= MAX_ID_LEN => Ok(s.to_string()),
                    _ => Err(bad("path holds non-empty strings")),
                })
                .collect::<Result<_, _>>()?
        }
    };
    Ok((id, path))
}

fn ids(params: &Params) -> Result<Vec<String>, FmError> {
    let map = object(params)?;
    let arr = map
        .and_then(|m| m.get("ids"))
        .and_then(Params::as_array)
        .ok_or_else(|| FmError::InvalidParams("ids must be an array of strings".to_string()))?;
    if arr.is_empty() || arr.len() > MAX_IDS {
        return Err(FmError::InvalidParams(format!(
            "ids must hold 1-{MAX_IDS} entries"
        )));
    }
    arr.iter()
        .map(|v| match v.as_str() {
            Some(s) if s.len() <= MAX_ID_LEN => Ok(s.to_string()),
            _ => Err(FmError::InvalidParams(
                "ids must be strings of at most 4096 bytes".to_string(),
            )),
        })
        .collect()
}

fn confirmed(params: &Params) -> Result<(), FmError> {
    let ok = object(params)?
        .and_then(|m| m.get("confirm"))
        .and_then(Params::as_bool)
        .unwrap_or(false);
    if ok {
        Ok(())
    } else {
        Err(FmError::InvalidParams(
            "permanent removal needs confirm: true".to_string(),
        ))
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "item" } else { "items" }
}

impl Session {
    fn trash_spawn<F>(&mut self, work: F) -> Result<(), FmError>
    where
        F: FnOnce() -> Outcome + Send + 'static,
    {
        if self.trash.busy {
            return Err(FmError::InvalidParams(
                "the trash is busy; try again in a moment".to_string(),
            ));
        }
        self.trash.busy = true;
        self.trash.error = None;
        let slot = self.trash.slot.0.clone();
        self.jobs.submit(JobLane::Local, move |_ctx| {
            let outcome = work();
            *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
            Ok(())
        });
        Ok(())
    }

    pub(crate) fn trash_list(&mut self) -> Result<(), FmError> {
        self.trash_spawn(|| Outcome::Listed(list_trash().map_err(|e| e.to_string())))
    }

    fn trash_load_children(&mut self, key: String, id: String, path: Vec<String>) {
        let slot = self.trash.child_slot.0.clone();
        self.jobs.submit(JobLane::Local, move |_ctx| {
            let result = list_trash_dir(&id, &path).map_err(|e| e.to_string());
            slot.lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((key, result));
            Ok(())
        });
    }

    fn trash_enter(&mut self, id: String, name: String, path: Vec<String>) {
        self.trash.view = Some(TrashView {
            id: id.clone(),
            name,
            path: path.clone(),
            children: Children::Loading,
        });
        self.trash_load_children(VIEW_KEY.to_string(), id, path);
    }

    fn pump_trash_children(&mut self) -> bool {
        let done: Vec<_> = std::mem::take(
            &mut *self
                .trash
                .child_slot
                .0
                .lock()
                .unwrap_or_else(|p| p.into_inner()),
        );
        let changed = !done.is_empty();
        for (key, result) in done {
            let state = match result {
                Ok(c) => Children::Ready(c),
                Err(e) => Children::Failed(e),
            };
            if key == VIEW_KEY {
                if let Some(v) = self.trash.view.as_mut() {
                    v.children = state;
                }
            } else if let Some(slot) = self.trash.expanded.get_mut(&key) {
                *slot = state;
            }
        }
        changed
    }

    /// Apply a finished trash job; returns whether the panel changed.
    pub(crate) fn pump_trash(&mut self) -> bool {
        let children = self.pump_trash_children();
        let outcome = self
            .trash
            .slot
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        let Some(outcome) = outcome else {
            return children;
        };
        self.trash.busy = false;
        self.trash.loaded = true;
        match outcome {
            Outcome::Listed(Ok(listing)) => {
                let alive: HashSet<&str> = listing.entries.iter().map(|e| e.id.as_str()).collect();
                self.trash.selected.retain(|id| alive.contains(id.as_str()));
                self.trash
                    .selected_children
                    .retain(|(id, _)| alive.contains(id.as_str()));
                self.trash
                    .expanded
                    .retain(|k, _| alive.contains(k.split(SEP).next().unwrap_or_default()));
                if self
                    .trash
                    .view
                    .as_ref()
                    .is_some_and(|v| !alive.contains(v.id.as_str()))
                {
                    self.trash.view = None;
                }
                self.trash.listing = listing;
            }
            Outcome::Listed(Err(e)) => self.trash.error = Some(e),
            Outcome::Acted(Ok(msg)) => {
                self.trash.notice = Some(msg);
                let _ = self.trash_list();
            }
            Outcome::Acted(Err(e)) => {
                self.trash.error = Some(e);
                let _ = self.trash_list();
            }
        }
        true
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        open(),
        refresh(),
        restore(),
        purge(),
        empty(),
        restore_children(),
        expand(),
        collapse(),
        enter(),
        leave(),
    ]
}

fn in_trash(s: &Session) -> bool {
    s.focused_kind() == Some(PaneKind::Trash)
}

/// `trash.open` — turn the focused pane into a Trash pane.
fn open() -> CommandSpec {
    CommandSpec {
        id: "trash.open",
        label: "Manage Trash",
        menu: &["File", "Manage Trash"],
        shortcut: None,
        params: "none",
        enabled: |_| true,
        run: |s, _p| {
            s.trash.notice = None;
            let pane = s.layout.focused();
            s.dispatch(
                "pane.kind.set",
                &serde_json::json!({"kind": "trash", "pane": pane}),
            )
        },
    }
}

fn refresh() -> CommandSpec {
    CommandSpec {
        id: "trash.refresh",
        label: "Refresh Trash",
        menu: &["File", "Manage Trash…", "Refresh"],
        shortcut: None,
        params: "none",
        enabled: |s| in_trash(s) && !s.trash.busy,
        run: |s, _p| s.trash_list(),
    }
}

fn restore() -> CommandSpec {
    CommandSpec {
        id: "trash.restore",
        label: "Restore from Trash",
        menu: &["File", "Manage Trash…", "Restore"],
        shortcut: None,
        params: "ids: [string] (trash entry ids)",
        enabled: |s| in_trash(s) && !s.trash.busy && !s.trash.selected.is_empty(),
        run: |s, p| {
            let ids = ids(p)?;
            s.trash_spawn(move || {
                Outcome::Acted(
                    restore_trash(&ids)
                        .map(|n| format!("Restored {n} {}", plural(n)))
                        .map_err(|e| e.to_string()),
                )
            })
        },
    }
}

fn purge() -> CommandSpec {
    CommandSpec {
        id: "trash.purge",
        label: "Delete Permanently from Trash",
        menu: &["File", "Manage Trash…", "Delete Permanently"],
        shortcut: None,
        params: "ids: [string], confirm: true",
        enabled: |s| in_trash(s) && !s.trash.busy && !s.trash.selected.is_empty(),
        run: |s, p| {
            confirmed(p)?;
            let ids = ids(p)?;
            s.trash_spawn(move || {
                Outcome::Acted(
                    purge_trash(&ids)
                        .map(|n| format!("Deleted {n} {} permanently", plural(n)))
                        .map_err(|e| e.to_string()),
                )
            })
        },
    }
}

fn empty() -> CommandSpec {
    CommandSpec {
        id: "trash.empty",
        label: "Empty Trash",
        menu: &["File", "Manage Trash…", "Empty Trash"],
        shortcut: None,
        params: "confirm: true",
        enabled: |s| in_trash(s) && !s.trash.busy && !s.trash.listing.entries.is_empty(),
        run: |s, p| {
            confirmed(p)?;
            s.trash_spawn(|| {
                Outcome::Acted(
                    empty_trash()
                        .map(|n| format!("Emptied the trash ({n} {})", plural(n)))
                        .map_err(|e| e.to_string()),
                )
            })
        },
    }
}

/// `trash.restore_children` — put items from inside trashed folders back in place.
fn restore_children() -> CommandSpec {
    CommandSpec {
        id: "trash.restore_children",
        label: "Restore Items from Trashed Folders",
        menu: &["File", "Manage Trash…", "Restore Inner Items"],
        shortcut: None,
        params: "items: [{id: string, path: [string]}]",
        enabled: |s| in_trash(s) && !s.trash.busy && !s.trash.selected_children.is_empty(),
        run: |s, p| {
            let bad = |m: &str| FmError::InvalidParams(m.to_string());
            let arr = object(p)?
                .and_then(|m| m.get("items"))
                .and_then(Params::as_array)
                .ok_or_else(|| bad("items is required"))?;
            if arr.is_empty() || arr.len() > MAX_IDS {
                return Err(bad("items must hold 1-100000 entries"));
            }
            let mut items = Vec::new();
            for v in arr {
                let (id, path) = target(v)?;
                if path.is_empty() {
                    return Err(bad("path must not be empty"));
                }
                let entry = s
                    .trash
                    .listing
                    .entries
                    .iter()
                    .find(|e| e.id == id)
                    .ok_or_else(|| bad("unknown trash entry"))?;
                items.push((id, path, entry.original_parent.clone(), entry.name.clone()));
            }
            s.trash.selected_children.clear();
            s.trash_spawn(move || {
                let mut done = 0usize;
                let mut failed = None;
                for (id, path, parent, name) in &items {
                    match restore_trash_child(id, path, parent, name) {
                        Ok(()) => done += 1,
                        Err(e) => {
                            failed = Some(e.to_string());
                            break;
                        }
                    }
                }
                Outcome::Acted(match failed {
                    None => Ok(format!("Restored {done} {}", plural(done))),
                    Some(e) => Err(format!("Restored {done}, then failed: {e}")),
                })
            })
        },
    }
}

/// `trash.expand` — show a trashed folder's contents under its row.
fn expand() -> CommandSpec {
    CommandSpec {
        id: "trash.expand",
        label: "Expand Trashed Folder",
        menu: &["File", "Manage Trash…", "Expand Folder"],
        shortcut: None,
        params: "id: string (trash entry id), path: [string] (folders inside it)",
        enabled: in_trash,
        run: |s, p| {
            let (id, path) = target(p)?;
            let key = child_key(&id, &path);
            if !matches!(s.trash.expanded.get(&key), Some(Children::Ready(_))) {
                s.trash.expanded.insert(key.clone(), Children::Loading);
                s.trash_load_children(key, id, path);
            }
            Ok(())
        },
    }
}

/// `trash.collapse` — hide an inline-expanded folder.
fn collapse() -> CommandSpec {
    CommandSpec {
        id: "trash.collapse",
        label: "Collapse Trashed Folder",
        menu: &["File", "Manage Trash…", "Collapse Folder"],
        shortcut: None,
        params: "id: string, path: [string]",
        enabled: in_trash,
        run: |s, p| {
            let (id, path) = target(p)?;
            let key = child_key(&id, &path);
            let prefix = format!("{key}{SEP}");
            s.trash
                .expanded
                .retain(|k, _| *k != key && !k.starts_with(&prefix));
            Ok(())
        },
    }
}

/// `trash.enter` — open a trashed folder (or a folder inside it) as the pane's listing.
fn enter() -> CommandSpec {
    CommandSpec {
        id: "trash.enter",
        label: "Open Trashed Folder",
        menu: &["File", "Manage Trash…", "Open Folder"],
        shortcut: None,
        params: "id: string, path: [string], name: string (optional, for the header)",
        enabled: in_trash,
        run: |s, p| {
            let (id, path) = target(p)?;
            let name = object(p)?
                .and_then(|m| m.get("name"))
                .and_then(Params::as_str)
                .map(|n| n.chars().take(256).collect::<String>())
                .or_else(|| path.last().cloned())
                .unwrap_or_default();
            s.trash_enter(id, name, path);
            Ok(())
        },
    }
}

/// `trash.leave` — go up one folder, back to the trash list from the top.
fn leave() -> CommandSpec {
    CommandSpec {
        id: "trash.leave",
        label: "Up in Trash",
        menu: &["File", "Manage Trash…", "Up"],
        shortcut: None,
        params: "none",
        enabled: |s| in_trash(s) && s.trash.view.is_some(),
        run: |s, _p| {
            let Some(mut v) = s.trash.view.take() else {
                return Ok(());
            };
            if v.path.pop().is_some() {
                let name = v.path.last().cloned().unwrap_or(v.name);
                s.trash_enter(v.id, name, v.path);
            }
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn malformed_params_are_rejected() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("trash.restore", json!({})),
            ("trash.restore", json!({"ids": []})),
            ("trash.restore", json!({"ids": [1]})),
            ("trash.restore", json!("x")),
            ("trash.purge", json!({"ids": ["a"]})),
            ("trash.purge", json!({"ids": ["a"], "confirm": "yes"})),
            ("trash.empty", json!({})),
            ("trash.empty", json!({"confirm": false})),
            ("trash.restore_children", json!({})),
            ("trash.restore_children", json!({"items": []})),
            (
                "trash.restore_children",
                json!({"items": [{"id": "a", "path": []}]}),
            ),
            ("trash.expand", json!({})),
            ("trash.expand", json!({"id": 3})),
            ("trash.expand", json!({"id": "a", "path": "x"})),
            ("trash.expand", json!({"id": "a", "path": [1]})),
            ("trash.collapse", json!({"id": ""})),
            ("trash.enter", json!({"path": ["a"]})),
            ("trash.enter", json!("x")),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
        assert!(!s.trash.busy, "rejected commands start no job");
    }

    #[test]
    fn open_turns_the_focused_pane_into_a_trash_pane() {
        let mut s = Session::new().expect("session");
        s.dispatch("trash.open", &json!({})).expect("open");
        assert_eq!(s.focused_kind(), Some(PaneKind::Trash));
    }
}
