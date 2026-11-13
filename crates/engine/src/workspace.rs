//! Workspace tabs (capability `workspaces`): named collections of panes, each with its own
//! layout, locations and per-pane settings.
//!
//! The *active* tab's layout is `Session::layout`, so every pane command keeps working
//! unchanged; inactive tabs park theirs in the tab. Per-pane engine state (terminals,
//! previews, view indexes) is keyed by `PaneId`, and every tab draws its ids from its own
//! range, so background tabs keep their shells alive. Up to [`MAX_SAVED`] tabs can be
//! saved; the saved set is the only thing written to disk (`workspaces.json`, owner-only,
//! URIs only — never a secret).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use fm_layout::{Layout, PaneId, PaneKind};
use fm_vfs::{FmError, Scheme, VfsUri};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::preview_cmds::{PreviewSlot, Zoom};
use crate::session::Session;

/// Most workspaces that can be saved (open tabs are not limited).
pub const MAX_SAVED: usize = 10;
/// Longest workspace name, in characters.
pub const NAME_MAX: usize = 40;
/// File name inside the config directory.
pub const WORKSPACES_FILE: &str = "workspaces.json";
/// Larger files are ignored (the built-in workspace opens instead).
pub const MAX_WORKSPACES_BYTES: u64 = 8 * 1024 * 1024;
const FORMAT_VERSION: u32 = 1;
/// Pane ids each tab may use before the next range starts.
const PANE_RANGE: PaneId = 1 << 20;

/// What a Preview pane remembers about its view (the content itself is re-rendered).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviewSettings {
    #[serde(default)]
    pub pretty: bool,
    #[serde(default)]
    pub wrap: bool,
    #[serde(default)]
    pub thumbs: bool,
    #[serde(default)]
    pub thumb_size: usize,
    #[serde(default = "zoom_fit")]
    pub zoom: Zoom,
}

fn zoom_fit() -> Zoom {
    Zoom::Fit
}

/// Everything a workspace restores: the pane tree with its state, plus the settings of the
/// panels that keep theirs in the engine.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Snapshot {
    layout: Layout,
    #[serde(default)]
    previews: BTreeMap<PaneId, PreviewSettings>,
}

/// A saved workspace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub name: String,
    /// Changes write back continuously; otherwise only an explicit save updates it.
    #[serde(default)]
    pub auto_save: bool,
    #[serde(default)]
    pub open_at_startup: bool,
    /// Opens on start when no workspace is marked for startup, and replaces the last tab
    /// when it is closed.
    #[serde(default)]
    pub default: bool,
    pub(crate) snapshot: Value,
    /// Runtime identity (not persisted); an open tab of this workspace carries the same.
    #[serde(skip)]
    pub uid: u64,
}

#[derive(Serialize, Deserialize)]
struct Doc {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    workspaces: Vec<Value>,
}

/// An open tab.
#[derive(Debug)]
pub struct Tab {
    pub uid: u64,
    pub name: String,
    /// Parked layout of an inactive tab; the active tab's lives in `Session::layout`.
    layout: Option<Layout>,
    /// Fingerprint of the state last saved (or first opened); a difference is "unsaved".
    baseline: String,
    /// The untouched built-in tab never asks to be saved.
    pub ephemeral: bool,
    /// Refreshed by [`Session::workspace_tick`] so the tab bar never serializes.
    pub dirty: bool,
}

/// The open tabs and the saved set.
#[derive(Debug)]
pub struct Workspaces {
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub saved: Vec<Saved>,
    next_uid: u64,
    next_base: PaneId,
    changed: bool,
}

/// What the unsaved-changes dialog is guarding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AfterPrompt {
    Quit,
    Close(u64),
}

/// Raised when quitting or closing would lose changes the user has not saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsavedPrompt {
    pub after: AfterPrompt,
    pub names: Vec<String>,
}

/// Which per-workspace flag a command sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    AutoSave,
    OpenAtStartup,
    Default,
}

impl Workspaces {
    /// One ephemeral tab holding the session's first layout.
    pub(crate) fn new(first: &Layout) -> Self {
        let baseline = serde_json::to_value(Snapshot {
            layout: first.clone(),
            previews: BTreeMap::new(),
        })
        .map(|v| fingerprint(&v))
        .unwrap_or_default();
        Self {
            tabs: vec![Tab {
                uid: 1,
                name: "Workspace 1".to_string(),
                layout: None,
                baseline,
                ephemeral: true,
                dirty: false,
            }],
            active: 0,
            saved: Vec::new(),
            next_uid: 2,
            next_base: PANE_RANGE,
            changed: false,
        }
    }

    pub fn saved_for(&self, uid: u64) -> Option<&Saved> {
        self.saved.iter().find(|s| s.uid == uid)
    }

    pub fn is_saved(&self, uid: u64) -> bool {
        self.saved_for(uid).is_some()
    }

    fn saved_ix(&self, uid: u64) -> Option<usize> {
        self.saved.iter().position(|s| s.uid == uid)
    }

    fn name_taken(&self, name: &str, except: Option<u64>) -> bool {
        let same = |n: &str| n.eq_ignore_ascii_case(name);
        self.tabs
            .iter()
            .any(|t| Some(t.uid) != except && same(&t.name))
            || self
                .saved
                .iter()
                .any(|s| Some(s.uid) != except && same(&s.name))
    }

    fn fresh_name(&self) -> String {
        (1u32..)
            .map(|n| format!("Workspace {n}"))
            .find(|n| !self.name_taken(n, None))
            .unwrap_or_else(|| "Workspace".to_string())
    }

    fn alloc_uid(&mut self) -> u64 {
        let uid = self.next_uid;
        self.next_uid += 1;
        uid
    }

    fn alloc_base(&mut self) -> PaneId {
        let base = self.next_base;
        self.next_base = self.next_base.saturating_add(PANE_RANGE);
        base
    }

    /// The saved workspace marked as default, if any.
    pub fn default_saved(&self) -> Option<&Saved> {
        self.saved.iter().find(|s| s.default)
    }
}

/// Validate and normalize a user-given name.
pub fn clean_name(raw: &str) -> Result<String, FmError> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
        return Err(FmError::InvalidParams(format!(
            "workspace name must be 1-{NAME_MAX} characters, without control characters"
        )));
    }
    Ok(name.to_string())
}

/// The snapshot without view noise (scroll offsets and one-shot "scroll to" flags), so
/// merely scrolling does not count as a change.
fn fingerprint(value: &Value) -> String {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for key in ["scroll", "reveal", "scroll_focus"] {
                    map.remove(key);
                }
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut v = value.clone();
    strip(&mut v);
    v.to_string()
}

/// One Files pane at home, with the tree rooted at the filesystem root.
fn home_layout(base: u64) -> Layout {
    Layout::single_rooted(home_location(), VfsUri::root_file(), base)
}

pub fn home_location() -> VfsUri {
    crate::goto_cmds::home_dir()
        .and_then(|h| VfsUri::parse(&h).ok())
        .unwrap_or_else(VfsUri::root_file)
}

impl Session {
    pub(crate) fn evacuate_volume(&mut self, mount: &VfsUri, home: &VfsUri) {
        fn evacuate(layout: &mut Layout, mount: &VfsUri, home: &VfsUri) {
            for id in layout.panes() {
                if let Some(pane) = layout.state_mut(id)
                    && mount.is_same_or_descendant_of(&pane.location)
                {
                    pane.navigate(home.clone());
                    if !pane.tree.root.is_same_or_descendant_of(home) {
                        pane.tree = fm_layout::TreeState::new(VfsUri::root_file());
                    }
                    pane.reveal_location();
                }
            }
        }
        evacuate(&mut self.layout, mount, home);
        for tab in &mut self.workspaces.tabs {
            if let Some(layout) = &mut tab.layout {
                evacuate(layout, mount, home);
            }
        }
    }
    fn tab_layout(&self, ix: usize) -> Option<&Layout> {
        if ix == self.workspaces.active {
            Some(&self.layout)
        } else {
            self.workspaces.tabs.get(ix)?.layout.as_ref()
        }
    }

    fn capture(&self, layout: &Layout) -> Snapshot {
        let previews = layout
            .states
            .iter()
            .filter(|(_, st)| st.kind == PaneKind::Preview)
            .filter_map(|(id, _)| {
                let slot = self.previews.slots.get(id)?;
                Some((
                    *id,
                    PreviewSettings {
                        pretty: slot.pretty,
                        wrap: slot.wrap,
                        thumbs: slot.thumbs,
                        thumb_size: slot.thumb_size,
                        zoom: slot.zoom,
                    },
                ))
            })
            .collect();
        Snapshot {
            layout: layout.clone(),
            previews,
        }
    }

    fn tab_value(&self, ix: usize) -> Option<Value> {
        let layout = self.tab_layout(ix)?;
        serde_json::to_value(self.capture(layout)).ok()
    }

    /// Whether tab `ix` differs from what was last saved (auto-saved tabs never do).
    fn compute_dirty(&self, ix: usize) -> bool {
        let Some(tab) = self.workspaces.tabs.get(ix) else {
            return false;
        };
        if tab.ephemeral
            || self
                .workspaces
                .saved_for(tab.uid)
                .is_some_and(|s| s.auto_save)
        {
            return false;
        }
        self.tab_value(ix)
            .is_some_and(|v| fingerprint(&v) != tab.baseline)
    }

    /// Write the live state of auto-save tabs back into the saved set.
    fn sync_autosaved(&mut self) {
        for ix in 0..self.workspaces.tabs.len() {
            let Some(uid) = self.workspaces.tabs.get(ix).map(|t| t.uid) else {
                continue;
            };
            let Some(sx) = self.workspaces.saved_ix(uid) else {
                continue;
            };
            if !self.workspaces.saved.get(sx).is_some_and(|s| s.auto_save) {
                continue;
            }
            if let Some(value) = self.tab_value(ix)
                && let Some(saved) = self.workspaces.saved.get_mut(sx)
                && saved.snapshot != value
            {
                saved.snapshot = value;
                self.workspaces.changed = true;
            }
        }
    }

    /// Periodic housekeeping (the shell calls it about once a second): refresh the
    /// unsaved markers and sync auto-save tabs. Returns the file text when the saved set
    /// changed and must be written.
    pub fn workspace_tick(&mut self) -> Option<String> {
        let dirty: Vec<bool> = (0..self.workspaces.tabs.len())
            .map(|ix| self.compute_dirty(ix))
            .collect();
        for (tab, d) in self.workspaces.tabs.iter_mut().zip(dirty) {
            tab.dirty = d;
        }
        self.sync_autosaved();
        if !self.workspaces.changed {
            return None;
        }
        self.workspaces.changed = false;
        self.workspace_file()
    }

    /// Ask for the saved set to be returned by the next tick again (a write failed).
    pub fn workspace_retry(&mut self) {
        self.workspaces.changed = true;
    }

    fn workspace_file(&self) -> Option<String> {
        let doc = Doc {
            version: FORMAT_VERSION,
            workspaces: self
                .workspaces
                .saved
                .iter()
                .filter_map(|s| serde_json::to_value(s).ok())
                .collect(),
        };
        serde_json::to_string(&doc).ok()
    }

    /// Names of open tabs with unsaved changes (computed now, not from the cache).
    pub fn workspace_unsaved(&self) -> Vec<(u64, String)> {
        (0..self.workspaces.tabs.len())
            .filter(|ix| self.compute_dirty(*ix))
            .filter_map(|ix| self.workspaces.tabs.get(ix))
            .map(|t| (t.uid, t.name.clone()))
            .collect()
    }

    /// Release what a layout's panes own (shells, previews, thumbnails, view indexes).
    fn forget_layout(&mut self, layout: &Layout) {
        for pane in layout.states.keys().copied() {
            self.drop_terminal(pane);
            self.previews.slots.remove(&pane);
            self.thumbs.cancel(pane);
            self.invalidate_view(pane);
        }
        // No thumbnail grid left anywhere: give back the in-memory thumbnails (the disk
        // cache still serves them when a grid opens again).
        if !self.previews.slots.values().any(|slot| slot.thumbs) {
            self.thumbs.clear_memory();
        }
    }

    fn kick_reads(&mut self) {
        // Unconnected SFTP panes fall back to Saved servers instead of asking to sign in.
        self.normalize_sftp_panes();
        self.media_visibility_changed();
        let locations: Vec<VfsUri> = self
            .layout
            .states
            .values()
            .filter(|st| {
                !matches!(
                    st.kind,
                    PaneKind::Terminal | PaneKind::Preview | PaneKind::Trash | PaneKind::Media
                )
            })
            .map(|st| st.location.clone())
            .collect();
        for uri in locations {
            let _ = self.ensure_read(uri);
        }
    }

    /// Rebuild a layout from a saved snapshot, on a fresh id range.
    fn restore(&mut self, value: &Value) -> Result<Layout, FmError> {
        let snap: Snapshot = serde_json::from_value(value.clone())
            .map_err(|e| FmError::InvalidParams(format!("saved workspace: {e}")))?;
        let Snapshot {
            mut layout,
            previews,
        } = snap;
        layout.validate()?;
        for st in layout.states.values_mut() {
            // SMB panes were folded into the Network pane.
            if st.kind == PaneKind::Smb {
                let net = VfsUri::network();
                st.kind = PaneKind::Network;
                st.tree = fm_layout::TreeState::new(net.clone());
                st.navigate(net);
            }
            // Only the selected folder is saved: local trees always start at `/`.
            if st.kind == PaneKind::Files
                && st.location.scheme() == Scheme::File
                && st.tree.root.scheme() == Scheme::File
                && st.tree.root != VfsUri::root_file()
            {
                st.tree = fm_layout::TreeState::new(VfsUri::root_file());
                st.reveal_location();
            }
        }
        layout.normalize_media_panes();
        let base = self.workspaces.alloc_base();
        let map = layout.relabel(base);
        for (old, settings) in previews {
            let Some(new) = map.get(&old) else { continue };
            if layout.state(*new).map(|s| s.kind) != Some(PaneKind::Preview) {
                continue;
            }
            let mut slot = PreviewSlot::new();
            slot.pretty = settings.pretty;
            slot.wrap = settings.wrap;
            slot.thumbs = settings.thumbs;
            slot.thumb_size = settings
                .thumb_size
                .min(fm_preview::thumbs::EDGES.len().saturating_sub(1));
            slot.zoom = settings.zoom;
            self.previews.slots.insert(*new, slot);
        }
        Ok(layout)
    }

    /// A brand-new tab layout: the default workspace when one is marked, else a single
    /// files pane at home.
    fn fresh_layout(&mut self) -> (Layout, Option<u64>) {
        let default = self
            .workspaces
            .default_saved()
            .map(|s| (s.snapshot.clone(), s.uid));
        if let Some((value, uid)) = default
            && !self.workspaces.tabs.iter().any(|t| t.uid == uid)
            && let Ok(layout) = self.restore(&value)
        {
            return (layout, Some(uid));
        }
        let base = self.workspaces.alloc_base();
        (home_layout(base), None)
    }

    fn push_tab(&mut self, layout: Layout, uid: Option<u64>, name: Option<String>) -> usize {
        let uid = uid.unwrap_or_else(|| self.workspaces.alloc_uid());
        let name = name
            .or_else(|| self.workspaces.saved_for(uid).map(|s| s.name.clone()))
            .unwrap_or_else(|| self.workspaces.fresh_name());
        let baseline = self
            .workspaces
            .saved_for(uid)
            .map(|s| fingerprint(&s.snapshot))
            .or_else(|| {
                serde_json::to_value(self.capture(&layout))
                    .ok()
                    .map(|v| fingerprint(&v))
            })
            .unwrap_or_default();
        self.workspaces.tabs.push(Tab {
            uid,
            name,
            layout: Some(layout),
            baseline,
            ephemeral: false,
            dirty: false,
        });
        self.workspaces.tabs.len() - 1
    }

    /// Make tab `ix` the active one.
    fn activate(&mut self, ix: usize) -> Result<(), FmError> {
        let active = self.workspaces.active;
        if ix == active {
            return Ok(());
        }
        let incoming = self
            .workspaces
            .tabs
            .get_mut(ix)
            .and_then(|t| t.layout.take())
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {ix}")))?;
        let outgoing = std::mem::replace(&mut self.layout, incoming);
        if let Some(tab) = self.workspaces.tabs.get_mut(active) {
            tab.layout = Some(outgoing);
        }
        self.workspaces.active = ix;
        self.path_edit = None;
        self.terminal_selection = None;
        self.pending_kind = None;
        self.kick_reads();
        Ok(())
    }

    /// `workspace.new`
    pub fn workspace_new(&mut self, name: Option<&str>) -> Result<(), FmError> {
        let name = name.map(clean_name).transpose()?;
        if let Some(n) = &name
            && self.workspaces.name_taken(n, None)
        {
            return Err(FmError::InvalidParams(format!("name in use: {n}")));
        }
        let base = self.workspaces.alloc_base();
        let layout = home_layout(base);
        let ix = self.push_tab(layout, None, name);
        self.activate(ix)
    }

    /// `workspace.switch` (0-based).
    pub fn workspace_switch(&mut self, ix: usize) -> Result<(), FmError> {
        if ix >= self.workspaces.tabs.len() {
            return Err(FmError::NotFound(format!("workspace tab {}", ix + 1)));
        }
        self.activate(ix)
    }

    /// Reorder tabs; the active tab stays active.
    pub fn workspace_move(&mut self, from: usize, to: usize) -> Result<(), FmError> {
        let n = self.workspaces.tabs.len();
        if from >= n || to >= n {
            return Err(FmError::NotFound("workspace tab".to_string()));
        }
        let active_uid = self
            .workspaces
            .tabs
            .get(self.workspaces.active)
            .map(|t| t.uid);
        let tab = self.workspaces.tabs.remove(from);
        self.workspaces.tabs.insert(to, tab);
        if let Some(uid) = active_uid
            && let Some(ix) = self.workspaces.tabs.iter().position(|t| t.uid == uid)
        {
            self.workspaces.active = ix;
        }
        Ok(())
    }

    /// Close tab `ix` without asking. The last tab is replaced by a fresh default.
    pub fn workspace_close_now(&mut self, ix: usize) -> Result<(), FmError> {
        if ix >= self.workspaces.tabs.len() {
            return Err(FmError::NotFound(format!("workspace tab {}", ix + 1)));
        }
        self.sync_autosaved();
        let active = self.workspaces.active;
        if self.workspaces.tabs.len() == 1 {
            let (layout, uid) = self.fresh_layout();
            let old = std::mem::replace(&mut self.layout, layout);
            self.forget_layout(&old);
            let fresh_name = self.workspaces.fresh_name();
            let saved_name = uid.and_then(|u| self.workspaces.saved_for(u).map(|s| s.name.clone()));
            let new_uid = uid.unwrap_or_else(|| self.workspaces.alloc_uid());
            let baseline = self
                .tab_value_of_active()
                .map(|v| fingerprint(&v))
                .unwrap_or_default();
            if let Some(tab) = self.workspaces.tabs.get_mut(0) {
                *tab = Tab {
                    uid: new_uid,
                    name: saved_name.unwrap_or(fresh_name),
                    layout: None,
                    baseline,
                    ephemeral: uid.is_none(),
                    dirty: false,
                };
            }
            self.kick_reads();
            return Ok(());
        }
        if ix == active {
            let next = if ix + 1 < self.workspaces.tabs.len() {
                ix + 1
            } else {
                ix - 1
            };
            self.activate(next)?;
        }
        let tab = self.workspaces.tabs.remove(ix);
        if let Some(layout) = tab.layout {
            self.forget_layout(&layout);
        }
        if let Some(a) = self.workspaces.tabs.iter().position(|t| t.layout.is_none()) {
            self.workspaces.active = a;
        }
        Ok(())
    }

    fn tab_value_of_active(&self) -> Option<Value> {
        self.tab_value(self.workspaces.active)
    }

    /// Close tab `ix`, asking first when it has unsaved changes.
    pub fn workspace_close(&mut self, ix: usize) -> Result<(), FmError> {
        let tab = self
            .workspaces
            .tabs
            .get(ix)
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?;
        if self.compute_dirty(ix) {
            self.unsaved_prompt = Some(UnsavedPrompt {
                after: AfterPrompt::Close(tab.uid),
                names: vec![tab.name.clone()],
            });
            return Ok(());
        }
        self.workspace_close_now(ix)
    }

    /// `app.quit`: stop now, or ask when workspaces have unsaved changes.
    pub fn request_quit(&mut self) {
        let unsaved = self.workspace_unsaved();
        if unsaved.is_empty() {
            self.running = false;
            return;
        }
        self.unsaved_prompt = Some(UnsavedPrompt {
            after: AfterPrompt::Quit,
            names: unsaved.into_iter().map(|(_, n)| n).collect(),
        });
    }

    /// Answer the unsaved-changes dialog: `save`, `discard` or `cancel`.
    pub fn workspace_resolve_prompt(&mut self, choice: &str) -> Result<(), FmError> {
        let Some(prompt) = self.unsaved_prompt.clone() else {
            return Ok(());
        };
        match choice {
            "cancel" => {}
            "discard" => self.after_prompt(&prompt.after)?,
            "save" => {
                let uids: Vec<u64> = match &prompt.after {
                    AfterPrompt::Close(uid) => vec![*uid],
                    AfterPrompt::Quit => self
                        .workspace_unsaved()
                        .into_iter()
                        .map(|(uid, _)| uid)
                        .collect(),
                };
                for uid in uids {
                    if let Some(ix) = self.workspaces.tabs.iter().position(|t| t.uid == uid) {
                        self.workspace_save(ix, None)?;
                    }
                }
                self.after_prompt(&prompt.after)?;
            }
            other => {
                return Err(FmError::InvalidParams(format!(
                    "choice must be save, discard or cancel, not {other:?}"
                )));
            }
        }
        self.unsaved_prompt = None;
        Ok(())
    }

    fn after_prompt(&mut self, after: &AfterPrompt) -> Result<(), FmError> {
        match after {
            AfterPrompt::Quit => {
                self.running = false;
                Ok(())
            }
            AfterPrompt::Close(uid) => {
                match self.workspaces.tabs.iter().position(|t| t.uid == *uid) {
                    Some(ix) => self.workspace_close_now(ix),
                    None => Ok(()),
                }
            }
        }
    }

    /// Save tab `ix` (optionally under `name`): creates the saved entry or updates it.
    pub fn workspace_save(&mut self, ix: usize, name: Option<&str>) -> Result<(), FmError> {
        if let Some(n) = name {
            self.workspace_rename(ix, n)?;
        }
        let value = self
            .tab_value(ix)
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?;
        let (uid, tab_name) = self
            .workspaces
            .tabs
            .get(ix)
            .map(|t| (t.uid, t.name.clone()))
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?;
        match self.workspaces.saved_ix(uid) {
            Some(sx) => {
                if let Some(s) = self.workspaces.saved.get_mut(sx) {
                    s.snapshot = value.clone();
                    s.name = tab_name;
                }
            }
            None => {
                if self.workspaces.saved.len() >= MAX_SAVED {
                    return Err(FmError::Unsupported(
                        "at most 10 workspaces can be saved; delete one first",
                    ));
                }
                self.workspaces.saved.push(Saved {
                    name: tab_name,
                    auto_save: false,
                    open_at_startup: false,
                    default: false,
                    snapshot: value.clone(),
                    uid,
                });
            }
        }
        if let Some(tab) = self.workspaces.tabs.get_mut(ix) {
            tab.baseline = fingerprint(&value);
            tab.ephemeral = false;
            tab.dirty = false;
        }
        self.workspaces.changed = true;
        Ok(())
    }

    pub fn workspace_rename(&mut self, ix: usize, raw: &str) -> Result<(), FmError> {
        let name = clean_name(raw)?;
        let uid = self
            .workspaces
            .tabs
            .get(ix)
            .map(|t| t.uid)
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?;
        if self.workspaces.name_taken(&name, Some(uid)) {
            return Err(FmError::InvalidParams(format!("name in use: {name}")));
        }
        if let Some(tab) = self.workspaces.tabs.get_mut(ix) {
            tab.name = name.clone();
            tab.ephemeral = false;
        }
        if let Some(sx) = self.workspaces.saved_ix(uid)
            && let Some(s) = self.workspaces.saved.get_mut(sx)
        {
            s.name = name;
            self.workspaces.changed = true;
        }
        Ok(())
    }

    /// Set a per-workspace flag, saving the tab first when it is not saved yet.
    pub fn workspace_set_flag(&mut self, ix: usize, flag: Flag, on: bool) -> Result<(), FmError> {
        let uid = self
            .workspaces
            .tabs
            .get(ix)
            .map(|t| t.uid)
            .ok_or_else(|| FmError::NotFound(format!("workspace tab {}", ix + 1)))?;
        if !self.workspaces.is_saved(uid) {
            self.workspace_save(ix, None)?;
        }
        self.set_saved_flag(uid, flag, on)
    }

    /// Set a flag on a saved workspace (open or not), addressed by its uid.
    pub fn set_saved_flag(&mut self, uid: u64, flag: Flag, on: bool) -> Result<(), FmError> {
        if flag == Flag::Default && on {
            for s in &mut self.workspaces.saved {
                s.default = false;
            }
        }
        let sx = self
            .workspaces
            .saved_ix(uid)
            .ok_or_else(|| FmError::NotFound("saved workspace".to_string()))?;
        if let Some(s) = self.workspaces.saved.get_mut(sx) {
            match flag {
                Flag::AutoSave => s.auto_save = on,
                Flag::OpenAtStartup => s.open_at_startup = on,
                Flag::Default => s.default = on,
            }
        }
        self.workspaces.changed = true;
        Ok(())
    }

    /// Open a saved workspace by name (switches to it when it is already open).
    pub fn workspace_open_saved(&mut self, name: &str) -> Result<(), FmError> {
        let (uid, value) = self
            .workspaces
            .saved
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .map(|s| (s.uid, s.snapshot.clone()))
            .ok_or_else(|| FmError::NotFound(format!("saved workspace {name:?}")))?;
        if let Some(ix) = self.workspaces.tabs.iter().position(|t| t.uid == uid) {
            return self.activate(ix);
        }
        let layout = self.restore(&value)?;
        let ix = self.push_tab(layout, Some(uid), None);
        self.activate(ix)
    }

    /// Remove a workspace from the saved set. An open tab stays open, unsaved.
    pub fn workspace_delete_saved(&mut self, name: &str) -> Result<(), FmError> {
        let sx = self
            .workspaces
            .saved
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| FmError::NotFound(format!("saved workspace {name:?}")))?;
        let removed = self.workspaces.saved.remove(sx);
        if let Some(ix) = self
            .workspaces
            .tabs
            .iter()
            .position(|t| t.uid == removed.uid)
            && let Some(tab) = self.workspaces.tabs.get_mut(ix)
        {
            tab.baseline.clear();
        }
        self.workspaces.changed = true;
        Ok(())
    }

    /// Replace the built-in tab with the workspaces marked for startup (or the default one).
    /// `text` is the content of `workspaces.json`; anything unreadable is skipped and the
    /// built-in workspace stays. Returns a notice for entries that could not be restored.
    pub fn workspace_startup(&mut self, text: &str) -> Option<String> {
        let doc: Doc = serde_json::from_str(text).ok()?;
        let mut skipped = 0usize;
        let mut saved: Vec<Saved> = Vec::new();
        for value in doc.workspaces.into_iter().take(MAX_SAVED) {
            match serde_json::from_value::<Saved>(value) {
                Ok(mut s) => match clean_name(&s.name) {
                    Ok(name) if !saved.iter().any(|o| o.name.eq_ignore_ascii_case(&name)) => {
                        s.name = name;
                        s.default &= !saved.iter().any(|o| o.default);
                        s.uid = self.workspaces.alloc_uid();
                        saved.push(s);
                    }
                    _ => skipped += 1,
                },
                Err(_) => skipped += 1,
            }
        }
        self.workspaces.saved = saved;

        let mut start: Vec<u64> = self
            .workspaces
            .saved
            .iter()
            .filter(|s| s.open_at_startup)
            .map(|s| s.uid)
            .collect();
        if start.is_empty()
            && let Some(d) = self.workspaces.default_saved()
        {
            start.push(d.uid);
        }
        let mut layouts: Vec<(u64, Layout)> = Vec::new();
        for uid in start {
            let Some(value) = self.workspaces.saved_for(uid).map(|s| s.snapshot.clone()) else {
                continue;
            };
            match self.restore(&value) {
                Ok(layout) => layouts.push((uid, layout)),
                Err(_) => skipped += 1,
            }
        }
        if !layouts.is_empty() {
            let active_uid = layouts
                .iter()
                .find(|(uid, _)| self.workspaces.saved_for(*uid).is_some_and(|s| s.default))
                .map(|(uid, _)| *uid)
                .or_else(|| layouts.first().map(|(uid, _)| *uid));
            let builtin = std::mem::replace(&mut self.layout, Layout::single(VfsUri::root_file()));
            self.forget_layout(&builtin);
            self.workspaces.tabs.clear();
            for (uid, layout) in layouts {
                self.push_tab(layout, Some(uid), None);
            }
            self.workspaces.active = self
                .workspaces
                .tabs
                .iter()
                .position(|t| Some(t.uid) == active_uid)
                .unwrap_or(0);
            let ix = self.workspaces.active;
            if let Some(layout) = self
                .workspaces
                .tabs
                .get_mut(ix)
                .and_then(|t| t.layout.take())
            {
                self.layout = layout;
            }
            self.kick_reads();
        }
        (skipped > 0).then(|| format!("{skipped} saved workspace(s) could not be restored"))
    }
}

/// Read the saved-workspaces file from the config directory (size-capped).
pub fn load_file(dir: &Path) -> Option<String> {
    let path = dir.join(WORKSPACES_FILE);
    if std::fs::metadata(&path).ok()?.len() > MAX_WORKSPACES_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Write the saved-workspaces file (owner-only, atomic).
pub fn save_file(dir: &Path, text: &str) -> Result<(), FmError> {
    crate::prefs::write_atomic(dir, WORKSPACES_FILE, text.as_bytes())
}

/// Pane ids of every parked (inactive) tab.
pub(crate) fn background_panes(ws: &Workspaces) -> BTreeSet<PaneId> {
    ws.tabs
        .iter()
        .filter_map(|t| t.layout.as_ref())
        .flat_map(|l| l.states.keys().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_layout_roots_the_tree_at_the_filesystem_root() {
        let l = home_layout(10);
        let st = l.state(l.focused()).expect("pane");
        assert_eq!(st.tree.root, VfsUri::root_file());
        assert_eq!(st.location, home_location());
    }

    #[test]
    fn legacy_smb_pane_restores_as_a_network_pane() {
        let mut s = crate::Session::new().expect("session");
        let mut layout = home_layout(10);
        let focused = layout.focused();
        if let Some(st) = layout.state_mut(focused) {
            st.kind = PaneKind::Smb;
        }
        let value = serde_json::to_value(Snapshot {
            layout,
            previews: BTreeMap::new(),
        })
        .expect("json");
        let restored = s.restore(&value).expect("restore");
        let st = restored.state(restored.focused()).expect("pane");
        assert_eq!(st.kind, PaneKind::Network);
        assert_eq!(st.location, VfsUri::network());
    }

    #[test]
    fn duplicate_media_panes_in_saved_workspace_restore_as_files() {
        let mut s = crate::Session::new().expect("session");
        let mut layout = home_layout(10);
        let first = layout.focused();
        let second = layout
            .split(first, fm_layout::Axis::Vertical, 0.5)
            .expect("split");
        layout.state_mut(first).expect("first").kind = PaneKind::Media;
        layout.state_mut(second).expect("second").kind = PaneKind::Media;
        let value = serde_json::to_value(Snapshot {
            layout,
            previews: BTreeMap::new(),
        })
        .expect("json");
        let restored = s.restore(&value).expect("restore");
        let panes = restored.panes();
        assert_eq!(
            restored.state(panes[0]).expect("first").kind,
            PaneKind::Media
        );
        assert_eq!(
            restored.state(panes[1]).expect("second").kind,
            PaneKind::Files
        );
    }

    #[test]
    fn restored_local_pane_roots_its_tree_at_the_filesystem_root() {
        let mut s = crate::Session::new().expect("session");
        let loc = VfsUri::parse("/home/u/code").expect("uri");
        let layout = Layout::single_rooted(loc.clone(), VfsUri::parse("/home/u").expect("uri"), 10);
        let value = serde_json::to_value(Snapshot {
            layout,
            previews: BTreeMap::new(),
        })
        .expect("json");
        let restored = s.restore(&value).expect("restore");
        let st = restored.state(restored.focused()).expect("pane");
        assert_eq!(st.tree.root, VfsUri::root_file());
        assert_eq!(st.location, loc);
        assert!(st.tree.is_expanded(&VfsUri::parse("/home/u").expect("uri")));
    }
}
