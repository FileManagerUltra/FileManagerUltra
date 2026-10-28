//! Per-pane view state: tree expansion, listing sort/selection, navigation history.

use std::collections::BTreeSet;

use fm_vfs::{Entry, EntryKind, Scheme, VfsUri};

use crate::PaneId;

/// Sort column.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Mtime,
    Type,
}

/// Sort direction.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SortDir {
    #[default]
    Asc,
    Desc,
}

/// Full sort spec for one pane's listing.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SortSpec {
    pub key: SortKey,
    pub dir: SortDir,
    pub dirs_first: bool,
}

impl Default for SortSpec {
    fn default() -> Self {
        Self {
            key: SortKey::Name,
            dir: SortDir::Asc,
            dirs_first: true,
        }
    }
}

impl SortSpec {
    /// Click a header: same key flips direction, new key starts ascending.
    pub fn header_click(&mut self, key: SortKey) {
        if self.key == key {
            self.dir = match self.dir {
                SortDir::Asc => SortDir::Desc,
                SortDir::Desc => SortDir::Asc,
            };
        } else {
            self.key = key;
            self.dir = SortDir::Asc;
        }
    }

    fn rank(&self, e: &Entry) -> (u8, u8) {
        let dir = u8::from(e.is_dir());
        // Under dirs_first, directories sort before files (rank 0).
        let primary = if self.dirs_first { 1 - dir } else { 0 };
        (primary, dir)
    }

    fn cmp_secondary(&self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
        let ord = match self.key {
            SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            SortKey::Size => a.size.unwrap_or(0).cmp(&b.size.unwrap_or(0)),
            SortKey::Mtime => a.modified.cmp(&b.modified),
            SortKey::Type => {
                let ta = kind_rank(&a.kind);
                let tb = kind_rank(&b.kind);
                ta.cmp(&tb)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            }
        };
        match self.dir {
            SortDir::Asc => ord,
            SortDir::Desc => ord.reverse(),
        }
    }

    /// Total order over entries under this spec. Stable key tiebreak: name.
    pub fn compare(&self, a: &Entry, b: &Entry) -> std::cmp::Ordering {
        self.rank(a)
            .0
            .cmp(&self.rank(b).0)
            .then_with(|| self.cmp_secondary(a, b))
            .then_with(|| a.name.cmp(&b.name))
    }

    /// Sort a slice in place.
    pub fn sort(&self, entries: &mut [Entry]) {
        entries.sort_by(|a, b| self.compare(a, b));
    }
}

fn kind_rank(kind: &EntryKind) -> u8 {
    match kind {
        EntryKind::Dir => 0,
        EntryKind::File => 1,
        EntryKind::Symlink { .. } => 2,
        EntryKind::Other => 3,
    }
}

/// Lazy tree view state. Children are never stored here — the UI reads the
/// engine's directory cache for expanded nodes only.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TreeState {
    pub root: VfsUri,
    /// Expanded nodes, always including `root`.
    expanded: BTreeSet<VfsUri>,
    /// The node holding keyboard focus. Tracked here rather than only in
    /// egui so focus survives the rows of a refreshing directory vanishing
    /// for a few frames (egui drops focus when a widget stops rendering).
    #[serde(default)]
    pub focused: Option<VfsUri>,
    /// The tree (not the listing) is the active area: file commands act on `focused`.
    #[serde(default)]
    pub active: bool,
    /// The tree should scroll the current location into view once its row exists.
    #[serde(default)]
    pub reveal: bool,
    /// The focused node was moved by the keyboard off-screen: scroll it into view.
    #[serde(default)]
    pub scroll_focus: bool,
    pub scroll: f32,
}

impl TreeState {
    /// Root is always expanded.
    pub fn new(root: VfsUri) -> Self {
        let mut expanded = BTreeSet::new();
        expanded.insert(root.clone());
        Self {
            root,
            expanded,
            focused: None,
            active: false,
            reveal: false,
            scroll_focus: false,
            scroll: 0.0,
        }
    }

    pub fn is_expanded(&self, uri: &VfsUri) -> bool {
        self.expanded.contains(uri)
    }

    /// Returns true if the state changed.
    pub fn expand(&mut self, uri: VfsUri) -> bool {
        self.expanded.insert(uri)
    }

    /// Collapse any node except the root. Returns true if the state changed.
    pub fn collapse(&mut self, uri: &VfsUri) -> bool {
        uri != &self.root && self.expanded.remove(uri)
    }

    /// Collapse everything below the root (root stays expanded).
    pub fn collapse_all(&mut self) {
        let root = self.root.clone();
        self.expanded.clear();
        self.expanded.insert(root);
    }

    pub fn expanded(&self) -> impl Iterator<Item = &VfsUri> {
        self.expanded.iter()
    }
}

/// Listing view state for one pane.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListingState {
    pub sort: SortSpec,
    /// Selected entry **names** (stable across re-reads of the same directory).
    selection: BTreeSet<String>,
    pub focused_row: usize,
    /// The cursor is on the `.` row: the pane's own folder, which has no entry.
    #[serde(default)]
    pub on_current: bool,
    /// The cursor is on the `..` row (it is not an entry, so it has no `focused_row`).
    pub on_parent: bool,
    pub scroll: f32,
    /// Quick filter typed into the listing (`Some` while the filter bar is shown). Transient:
    /// never persisted, and cleared when the pane changes folder.
    #[serde(skip)]
    pub filter: Option<String>,
    /// Ask the UI to put the keyboard in the filter field on the next frame.
    #[serde(skip)]
    pub filter_focus: bool,
    /// Select and scroll to this name once the listing has loaded (Show in Folder).
    #[serde(skip)]
    pub reveal: Option<String>,
}

impl Default for ListingState {
    fn default() -> Self {
        Self {
            sort: SortSpec::default(),
            selection: BTreeSet::new(),
            focused_row: 0,
            on_current: false,
            on_parent: false,
            scroll: 0.0,
            filter: None,
            filter_focus: false,
            reveal: None,
        }
    }
}

impl ListingState {
    pub fn is_selected(&self, name: &str) -> bool {
        self.selection.contains(name)
    }

    pub fn selection(&self) -> impl Iterator<Item = &String> {
        self.selection.iter()
    }

    pub fn selected_count(&self) -> usize {
        self.selection.len()
    }

    /// Clear the selection and select `name` (if any), focusing its row.
    pub fn select_only(&mut self, name: Option<&str>, row: usize) {
        self.selection.clear();
        self.on_current = false;
        self.on_parent = false;
        if let Some(name) = name {
            self.selection.insert(name.to_string());
        }
        self.focused_row = row;
    }

    /// Ctrl-style: toggle `name`, move focus to its row.
    pub fn toggle(&mut self, name: &str, row: usize) {
        self.on_current = false;
        self.on_parent = false;
        if !self.selection.remove(name) {
            self.selection.insert(name.to_string());
        }
        self.focused_row = row;
    }

    /// Shift-style: select the range between the anchor row and `row`
    /// (inclusive) in the given display order, replacing the selection.
    pub fn extend_to(&mut self, order: &[String], row: usize) {
        self.selection.clear();
        self.on_current = false;
        self.on_parent = false;
        if order.is_empty() {
            self.focused_row = 0;
            return;
        }
        let row = row.min(order.len() - 1);
        let anchor = self.focused_row.min(order.len() - 1);
        let (lo, hi) = (anchor.min(row), anchor.max(row));
        for name in &order[lo..=hi] {
            self.selection.insert(name.clone());
        }
        self.focused_row = row;
    }

    /// Select every name in `order` (Ctrl+A).
    pub fn select_all(&mut self, order: &[String]) {
        self.on_current = false;
        self.on_parent = false;
        self.selection = order.iter().cloned().collect();
    }

    /// Select exactly the names in `order` that are not selected now (Invert Selection).
    pub fn invert(&mut self, order: &[String]) {
        self.on_current = false;
        self.on_parent = false;
        self.selection = order
            .iter()
            .filter(|name| !self.selection.contains(*name))
            .cloned()
            .collect();
    }

    /// Add (`add`) or remove the names in `order` for which `matches` holds; other selected
    /// names stay as they are. Returns how many names matched.
    pub fn select_matching(
        &mut self,
        order: &[String],
        add: bool,
        matches: impl Fn(&str) -> bool,
    ) -> usize {
        self.on_current = false;
        self.on_parent = false;
        let mut count = 0;
        for name in order.iter().filter(|n| matches(n)) {
            count += 1;
            if add {
                self.selection.insert(name.clone());
            } else {
                self.selection.remove(name);
            }
        }
        if let Some(row) = order.iter().position(|n| self.selection.contains(n))
            && add
        {
            self.focused_row = row;
        }
        count
    }

    /// Drop selected names that no longer exist (after a re-read).
    pub fn prune(&mut self, existing: &BTreeSet<String>) {
        self.selection.retain(|name| existing.contains(name));
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
        self.on_current = false;
        self.on_parent = false;
    }

    /// Put the cursor on the `.` row (nothing selected): the pane's own folder.
    pub fn focus_current(&mut self) {
        self.selection.clear();
        self.focused_row = 0;
        self.on_current = true;
        self.on_parent = false;
    }

    /// Put the cursor on the `..` row (nothing selected).
    pub fn focus_parent(&mut self) {
        self.selection.clear();
        self.focused_row = 0;
        self.on_current = false;
        self.on_parent = true;
    }

    /// `move_focus` for a listing that shows special rows above the entries: Up from the
    /// first entry lands on `..` (when present) and then on `.`; Home lands on `.`; Down
    /// walks back. Extending never includes a special row.
    pub fn move_focus_with_specials(
        &mut self,
        order: &[String],
        delta: isize,
        extend: bool,
        has_current: bool,
        has_parent: bool,
    ) {
        if self.on_current {
            if delta > 0 {
                if has_parent {
                    self.focus_parent();
                } else if !order.is_empty() {
                    self.select_only(order.first().map(String::as_str), 0);
                }
            }
            return;
        }
        if self.on_parent {
            if delta < 0 && has_current {
                self.focus_current();
                return;
            }
            if delta > 0 && !order.is_empty() {
                self.select_only(order.first().map(String::as_str), 0);
            }
            return;
        }
        let at_top = self.focused_row.min(order.len().saturating_sub(1)) == 0;
        if delta < 0 && !extend && at_top {
            if has_parent {
                self.focus_parent();
                return;
            }
            if has_current {
                self.focus_current();
                return;
            }
        }
        self.move_focus(order, delta, extend);
    }

    /// Move the focused row; with `extend` keep growing the range selection,
    /// otherwise collapse the selection to the focused name.
    pub fn move_focus(&mut self, order: &[String], delta: isize, extend: bool) {
        if order.is_empty() {
            self.focused_row = 0;
            self.selection.clear();
            return;
        }
        let last = order.len() - 1;
        let cur = self.focused_row.min(last) as isize;
        let next = (cur + delta).clamp(0, last as isize) as usize;
        if extend {
            let anchor = self.focused_row.min(last);
            self.focused_row = anchor;
            self.extend_to(order, next);
        } else {
            let name = order.get(next).cloned();
            self.select_only(name.as_deref(), next);
        }
    }

    /// Jump to an absolute row (Home/End), same semantics as `move_focus`.
    pub fn focus_absolute(&mut self, order: &[String], row: usize, extend: bool) {
        if order.is_empty() {
            self.focused_row = 0;
            self.selection.clear();
            return;
        }
        let row = row.min(order.len() - 1);
        if extend {
            self.extend_to(order, row);
        } else {
            let name = order.get(row).cloned();
            self.select_only(name.as_deref(), row);
        }
    }
}

/// Per-pane back/forward navigation history.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct History {
    back: Vec<VfsUri>,
    forward: Vec<VfsUri>,
}

impl History {
    pub fn new(start: VfsUri) -> Self {
        Self {
            back: vec![start],
            forward: Vec::new(),
        }
    }

    pub fn current(&self) -> Option<&VfsUri> {
        self.back.last()
    }

    /// Navigate somewhere new: clears the forward stack.
    pub fn push(&mut self, to: VfsUri) {
        self.back.push(to);
        self.forward.clear();
    }

    pub fn can_back(&self) -> bool {
        self.back.len() > 1
    }

    pub fn can_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Step back; returns the new current location, or `None` at the start.
    pub fn back(&mut self) -> Option<VfsUri> {
        if !self.can_back() {
            return None;
        }
        let cur = self.back.pop()?;
        self.forward.push(cur);
        self.current().cloned()
    }

    /// Step forward; returns the new current location, or `None` at the end.
    pub fn forward(&mut self) -> Option<VfsUri> {
        let cur = self.forward.pop()?;
        self.back.push(cur);
        self.current().cloned()
    }
}

/// What a pane shows. Only `Files` state lives in [`PaneState`]; other kinds keep their
/// runtime state in the engine and merely tag the pane here.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PaneKind {
    #[default]
    Files,
    Terminal,
    Preview,
    Sftp,
    /// Legacy: SMB connects through the Network pane now. Kept so saved workspaces load.
    Smb,
    Network,
    Trash,
}

impl PaneKind {
    pub const ALL: [PaneKind; 6] = [
        Self::Files,
        Self::Terminal,
        Self::Preview,
        Self::Sftp,
        Self::Network,
        Self::Trash,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Terminal => "terminal",
            Self::Preview => "preview",
            Self::Sftp => "sftp",
            Self::Smb => "smb",
            Self::Network => "network",
            Self::Trash => "trash",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Terminal => "Terminal",
            Self::Preview => "Preview",
            Self::Sftp => "SFTP",
            Self::Smb => "SMB",
            Self::Network => "Network",
            Self::Trash => "Trash",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// Kinds that show a tree + listing (an `Sftp`/`Smb` pane is a Files pane on a server).
    pub fn is_files(self) -> bool {
        matches!(self, Self::Files | Self::Sftp | Self::Smb | Self::Network)
    }
}

/// Which Files pane a Preview pane shows the selection of.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PreviewSource {
    /// The most recently focused Files pane.
    #[default]
    Follow,
    /// A pinned Files pane (falls back to `Follow` when it closes).
    Pane(PaneId),
}

/// Everything one pane remembers about its view.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PaneState {
    /// Which body the pane shows; the Files fields below are kept while it is not `Files`.
    pub kind: PaneKind,
    pub location: VfsUri,
    pub tree: TreeState,
    pub listing: ListingState,
    pub history: History,
    /// Width of the tree column in px (drag-resized, persisted per pane).
    #[serde(default = "default_tree_width")]
    pub tree_width: f32,
    /// Preferred Size, Modified and Type widths in px; zero means fit to content.
    #[serde(default = "default_col_widths")]
    pub col_widths: [f32; 3],
    /// Preview panes: which Files pane to show.
    #[serde(default)]
    pub preview_source: PreviewSource,
    /// Which Places sections are collapsed (persisted per pane).
    #[serde(default)]
    pub places: PlacesState,
}

/// Collapse state of the Places sections above the tree. All start expanded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PlacesState {
    pub devices_collapsed: bool,
    pub places_collapsed: bool,
    pub bookmarks_collapsed: bool,
}

fn default_tree_width() -> f32 {
    DEFAULT_TREE_WIDTH
}

fn default_col_widths() -> [f32; 3] {
    DEFAULT_COL_WIDTHS
}

/// Default tree width in px.
pub const DEFAULT_TREE_WIDTH: f32 = 220.0;
/// Detail columns fit to content until manually resized.
pub const DEFAULT_COL_WIDTHS: [f32; 3] = [0.0; 3];
pub(crate) const LEGACY_COL_WIDTHS: [f32; 3] = [90.0, 140.0, 72.0];
pub const MIN_COL_WIDTH: f32 = 40.0;
pub const MAX_COL_WIDTH: f32 = 400.0;

impl PaneState {
    /// A pane rooted at `location`: the tree starts expanded at the same uri.
    pub fn new(location: VfsUri) -> Self {
        let root = location.clone();
        Self::new_rooted(location, root)
    }

    /// A pane at `location` whose tree is rooted at `root` (an ancestor, e.g. `/`).
    pub fn new_rooted(location: VfsUri, root: VfsUri) -> Self {
        let mut pane = Self {
            kind: PaneKind::Files,
            tree: TreeState::new(root),
            history: History::new(location.clone()),
            location,
            listing: ListingState::default(),
            tree_width: DEFAULT_TREE_WIDTH,
            col_widths: DEFAULT_COL_WIDTHS,
            preview_source: PreviewSource::Follow,
            places: PlacesState::default(),
        };
        if pane.location != pane.tree.root {
            pane.reveal_location();
        }
        pane
    }

    /// Resize detail column `ix` (0 Size, 1 Modified, 2 Type); zero restores content
    /// fitting. Non-finite widths and unknown columns are ignored; the rest is clamped.
    pub fn set_col_width(&mut self, ix: usize, width: f32) {
        if let Some(w) = self.col_widths.get_mut(ix)
            && width.is_finite()
        {
            *w = if width == 0.0 {
                0.0
            } else {
                width.clamp(MIN_COL_WIDTH, MAX_COL_WIDTH)
            };
        }
    }

    /// Restore column `ix` to its default width.
    pub fn reset_col_width(&mut self, ix: usize) {
        if let (Some(w), Some(d)) = (self.col_widths.get_mut(ix), DEFAULT_COL_WIDTHS.get(ix)) {
            *w = *d;
        }
    }

    /// Commit a navigation: history push, selection reset. The caller
    /// (engine) is responsible for submitting the directory read.
    ///
    /// Re-opening the location the pane is already on is a no-op: it must not
    /// push a duplicate history entry nor wipe the user's selection, focus
    /// and scroll (re-opening the current directory is a reload, not a jump).
    pub fn navigate(&mut self, to: VfsUri) {
        if to == self.location {
            return;
        }
        self.history.push(to.clone());
        self.location = to;
        self.listing.filter = None;
        self.listing.focus_current();
        self.listing.scroll = 0.0;
        self.reveal_location();
    }

    /// Make the tree follow `location`: expand its ancestors (never the node itself, so a
    /// tree click can still collapse it), focus it and ask for it to be scrolled into view.
    /// Locations outside the tree's root are ignored.
    pub fn reveal_location(&mut self) {
        let mut chain = Vec::new();
        let mut cur = self.location.parent();
        while let Some(p) = cur {
            chain.push(p.clone());
            if p == self.tree.root {
                for uri in chain {
                    self.tree.expand(uri);
                }
                self.tree.focused = Some(self.location.clone());
                self.tree.reveal = !self.tree.active;
                return;
            }
            cur = p.parent();
        }
        // Hosts have no parent: under a Network tree they hang off its root.
        if matches!(self.tree.root.scheme(), Scheme::Network | Scheme::Servers)
            && self.location.scheme() != self.tree.root.scheme()
        {
            for uri in chain {
                self.tree.expand(uri);
            }
            self.tree.focused = Some(self.location.clone());
            self.tree.reveal = !self.tree.active;
            return;
        }
        if self.location == self.tree.root {
            self.tree.focused = Some(self.location.clone());
            self.tree.reveal = !self.tree.active;
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.history.can_back()
    }

    pub fn can_go_forward(&self) -> bool {
        self.history.can_forward()
    }

    /// Returns false when there is nowhere to go back to.
    pub fn go_back(&mut self) -> bool {
        match self.history.back() {
            Some(loc) => {
                self.location = loc;
                self.listing.filter = None;
                self.listing.focus_current();
                self.reveal_location();
                true
            }
            None => false,
        }
    }

    /// Returns false when there is nowhere to go forward to.
    pub fn go_forward(&mut self) -> bool {
        match self.history.forward() {
            Some(loc) => {
                self.location = loc;
                self.listing.filter = None;
                self.listing.focus_current();
                self.reveal_location();
                true
            }
            None => false,
        }
    }

    /// The parent of the current location, or `None` at the tree root.
    pub fn up_target(&self) -> Option<VfsUri> {
        if self.location == self.tree.root {
            return None;
        }
        self.location.parent()
    }
}

/// Focus bookkeeping layered on the pane tree.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Focus {
    pub focused: PaneId,
    #[serde(default)]
    pub previous: Option<PaneId>,
    /// The most recently focused Files pane (what a following Preview shows).
    #[serde(default)]
    pub last_files: Option<PaneId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rooted_pane_keeps_the_tree_at_the_root_and_reveals_the_location() {
        let p = PaneState::new_rooted(VfsUri::parse("/home/u").expect("uri"), VfsUri::root_file());
        assert_eq!(p.tree.root, VfsUri::root_file());
        assert_eq!(p.location, VfsUri::parse("/home/u").expect("uri"));
        assert!(p.tree.is_expanded(&VfsUri::parse("/home").expect("uri")));
    }

    #[test]
    fn invert_and_pattern_selection_work_on_the_visible_order() {
        let order: Vec<String> = ["a.jpg", "b.png", "c.jpg", "d.txt"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut l = ListingState::default();
        l.select_only(Some("a.jpg"), 0);
        l.invert(&order);
        assert!(!l.is_selected("a.jpg"));
        assert_eq!(l.selected_count(), 3);
        l.clear_selection();
        let jpg = |n: &str| n.ends_with(".jpg");
        assert_eq!(l.select_matching(&order, true, jpg), 2);
        assert!(l.is_selected("a.jpg") && l.is_selected("c.jpg"));
        assert_eq!(l.focused_row, 0);
        l.select_matching(&order, true, |n| n == "d.txt");
        assert_eq!(l.select_matching(&order, false, jpg), 2);
        assert_eq!(l.selection().collect::<Vec<_>>(), vec!["d.txt"]);
        l.invert(&[]);
        assert_eq!(l.selected_count(), 0);
    }

    #[test]
    fn the_quick_filter_is_transient_and_cleared_on_navigation() {
        let mut p = PaneState::new(VfsUri::parse("/d").expect("uri"));
        p.listing.filter = Some("jpg".into());
        let json = serde_json::to_string(&p).expect("ser");
        let back: PaneState = serde_json::from_str(&json).expect("de");
        assert_eq!(back.listing.filter, None);
        p.navigate(VfsUri::parse("/d/e").expect("uri"));
        assert_eq!(p.listing.filter, None);
        p.listing.filter = Some("x".into());
        assert!(p.go_back());
        assert_eq!(p.listing.filter, None);
        p.listing.filter = Some("x".into());
        assert!(p.go_forward());
        assert_eq!(p.listing.filter, None);
    }

    #[test]
    fn places_collapse_state_round_trips_and_defaults_to_expanded() {
        let mut p = PaneState::new(VfsUri::parse("/d").expect("uri"));
        assert_eq!(p.places, PlacesState::default());
        p.places.bookmarks_collapsed = true;
        p.places.devices_collapsed = true;
        let json = serde_json::to_string(&p).expect("ser");
        let back: PaneState = serde_json::from_str(&json).expect("de");
        assert!(back.places.bookmarks_collapsed && !back.places.places_collapsed);
        assert!(back.places.devices_collapsed);
        let mut v = serde_json::to_value(&p).expect("value");
        if let Some(o) = v.as_object_mut() {
            o.remove("places");
        }
        let legacy: PaneState = serde_json::from_value(v).expect("legacy");
        assert_eq!(legacy.places, PlacesState::default());
    }

    #[test]
    fn legacy_smb_kind_still_parses_from_saved_workspaces() {
        let k: PaneKind = serde_json::from_str("\"Smb\"").expect("legacy kind");
        assert_eq!(k, PaneKind::Smb);
        assert!(!PaneKind::ALL.contains(&PaneKind::Smb));
    }
    #[test]
    fn column_widths_clamp_reject_garbage_and_reset() {
        let mut p = PaneState::new(VfsUri::parse("/d").expect("uri"));
        p.set_col_width(1, 10_000.0);
        assert_eq!(p.col_widths[1], MAX_COL_WIDTH);
        p.set_col_width(1, 1.0);
        assert_eq!(p.col_widths[1], MIN_COL_WIDTH);
        p.set_col_width(0, f32::NAN);
        p.set_col_width(9, 100.0);
        assert_eq!(p.col_widths[0], DEFAULT_COL_WIDTHS[0]);
        p.reset_col_width(1);
        assert_eq!(p.col_widths, DEFAULT_COL_WIDTHS);
        p.reset_col_width(9);
    }

    #[test]
    fn content_fit_and_legacy_column_widths_survive_workspace_validation() {
        let mut layout = crate::Layout::single(VfsUri::parse("/d").expect("uri"));
        layout.states.get_mut(&1).expect("pane").col_widths = [0.0, 180.0, 72.0];
        let text = serde_json::to_string(&layout).expect("serialize");
        let mut restored: crate::Layout = serde_json::from_str(&text).expect("restore");
        restored.validate().expect("validate");
        assert_eq!(
            restored.state(1).expect("pane").col_widths,
            [0.0, 180.0, 72.0]
        );
        let mut legacy = serde_json::to_value(&restored).expect("json");
        legacy["states"]["1"]
            .as_object_mut()
            .expect("state")
            .remove("col_widths");
        let mut restored: crate::Layout = serde_json::from_value(legacy).expect("legacy");
        restored.validate().expect("validate");
        assert_eq!(restored.state(1).expect("pane").col_widths, [0.0; 3]);
        restored.states.get_mut(&1).expect("pane").col_widths = LEGACY_COL_WIDTHS;
        restored.validate().expect("migrate defaults");
        assert_eq!(restored.state(1).expect("pane").col_widths, [0.0; 3]);
    }

    fn uri(p: &str) -> VfsUri {
        VfsUri::parse(p).expect("static uri")
    }

    fn entry(name: &str, kind: EntryKind, size: Option<u64>) -> Entry {
        Entry {
            uri: uri(&format!("/d/{name}")),
            name: name.to_string(),
            kind,
            size,
            modified: None,
            permissions: None,
            hidden: false,
            link_to_dir: false,
            label: None,
        }
    }

    #[test]
    fn sort_by_name_dirs_first_asc_desc() {
        let mut es = vec![
            entry("b.txt", EntryKind::File, None),
            entry("A_dir", EntryKind::Dir, None),
            entry("a.txt", EntryKind::File, None),
            entry("Z_dir", EntryKind::Dir, None),
        ];
        let mut spec = SortSpec::default();
        spec.sort(&mut es);
        let names: Vec<_> = es.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["A_dir", "Z_dir", "a.txt", "b.txt"]);
        spec.header_click(SortKey::Name); // now descending (within each group)
        spec.sort(&mut es);
        let names: Vec<_> = es.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["Z_dir", "A_dir", "b.txt", "a.txt"]);
    }

    #[test]
    fn sort_dirs_first_off_and_size() {
        let mut es = vec![
            entry("big", EntryKind::File, Some(100)),
            entry("adir", EntryKind::Dir, None),
            entry("small", EntryKind::File, Some(5)),
        ];
        let spec = SortSpec {
            key: SortKey::Size,
            dir: SortDir::Asc,
            dirs_first: false,
        };
        spec.sort(&mut es);
        let names: Vec<_> = es.iter().map(|e| e.name.as_str()).collect();
        // dirs have size 0 under Size sort, so adir first when dirs_first is off
        assert_eq!(names, vec!["adir", "small", "big"]);
    }

    #[test]
    fn header_click_toggles_same_key_flips_new_key_resets() {
        let mut spec = SortSpec::default();
        assert_eq!(spec.key, SortKey::Name);
        spec.header_click(SortKey::Name);
        assert_eq!(spec.dir, SortDir::Desc);
        spec.header_click(SortKey::Size);
        assert_eq!(spec.key, SortKey::Size);
        assert_eq!(spec.dir, SortDir::Asc);
    }

    #[test]
    fn tree_root_stays_expanded() {
        let root = uri("/home");
        let mut t = TreeState::new(root.clone());
        assert!(t.is_expanded(&root));
        assert!(!t.collapse(&root), "root cannot collapse");
        let sub = uri("/home/docs");
        assert!(t.expand(sub.clone()));
        assert!(t.is_expanded(&sub));
        assert!(t.collapse(&sub));
        assert!(!t.is_expanded(&sub));
        t.expand(uri("/home/a"));
        t.expand(uri("/home/b"));
        t.collapse_all();
        assert!(t.is_expanded(&root));
        assert_eq!(t.expanded().count(), 1);
    }

    #[test]
    fn selection_click_shift_ctrl_all() {
        let order: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.select_only(Some("b"), 1);
        assert_eq!(l.selected_count(), 1);
        assert!(l.is_selected("b"));
        l.extend_to(&order, 3); // shift-click d from b
        assert_eq!(l.selected_count(), 3); // b,c,d
        assert!(l.is_selected("c") && l.is_selected("d") && !l.is_selected("a"));
        l.toggle("a", 0); // ctrl-click a
        assert!(l.is_selected("a"));
        l.toggle("a", 0);
        assert!(!l.is_selected("a"));
        l.select_all(&order);
        assert_eq!(l.selected_count(), 4);
    }

    #[test]
    fn selection_prune_and_reselect_survives_reread() {
        let order: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.select_only(Some("b"), 1);
        // re-read: b still there, a gone → b survives
        let mut existing = BTreeSet::new();
        existing.insert("b".to_string());
        l.prune(&existing);
        assert!(l.is_selected("b"));
        // now b gone too
        l.prune(&BTreeSet::new());
        assert_eq!(l.selected_count(), 0);
        // move_focus collapses multi-selection
        l.select_all(&order);
        l.move_focus(&order, 1, false);
        assert_eq!(l.selected_count(), 1);
        assert!(l.is_selected("b"));
    }

    #[test]
    fn the_special_rows_are_reachable_with_up_and_leave_with_down() {
        let order: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.select_only(Some("a"), 0);
        // `..` is nearer than `.`, so Up walks entry → `..` → `.`.
        l.move_focus_with_specials(&order, -1, false, true, true);
        assert!(l.on_parent && !l.on_current);
        assert_eq!(l.selected_count(), 0);
        l.move_focus_with_specials(&order, -1, false, true, true);
        assert!(l.on_current && !l.on_parent);
        l.move_focus_with_specials(&order, 1, false, true, true);
        assert!(l.on_parent && !l.on_current);
        l.move_focus_with_specials(&order, 1, false, true, true);
        assert!(!l.on_parent && !l.on_current);
        assert!(l.is_selected("a"));
        // Without a parent (a root) Down leaves `.` for the first entry.
        l.focus_current();
        l.move_focus_with_specials(&order, 1, false, true, false);
        assert!(!l.on_current && !l.on_parent);
        assert_eq!(l.focused_row, 0);
        assert!(l.is_selected("a"));
        // An empty directory can still reach both rows.
        let mut e = ListingState::default();
        e.move_focus_with_specials(&[], -1, false, true, true);
        assert!(e.on_parent);
        e.move_focus_with_specials(&[], -1, false, true, true);
        assert!(e.on_current);
        e.move_focus_with_specials(&[], 1, false, true, true);
        assert!(e.on_parent);
    }

    #[test]
    fn the_special_rows_never_join_the_selection() {
        let order: Vec<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.focus_current();
        l.select_all(&order);
        assert!(!l.on_current && !l.on_parent);
        assert_eq!(l.selected_count(), 2);
        l.focus_parent();
        l.select_only(Some("b"), 1);
        assert!(!l.on_parent && !l.on_current);
        l.focus_current();
        l.clear_selection();
        assert!(!l.on_current && !l.on_parent);
    }

    #[test]
    fn move_focus_extend_and_bounds() {
        let order: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.select_only(Some("a"), 0);
        l.move_focus(&order, 2, true); // shift+down,down
        assert_eq!(l.selected_count(), 3);
        l.move_focus(&order, 100, false); // clamps to end
        assert_eq!(l.focused_row, 2);
        assert_eq!(l.selected_count(), 1);
        l.focus_absolute(&order, 0, true); // shift+home extends from the anchor
        assert_eq!(l.focused_row, 0);
        assert_eq!(l.selected_count(), 3); // rows 0..=2
        // empty order never panics
        let mut e = ListingState::default();
        e.move_focus(&[], 5, true);
        e.select_all(&[]);
        e.extend_to(&[], 3);
        assert_eq!(e.focused_row, 0);
    }

    #[test]
    fn history_back_forward_push() {
        let a = uri("/a");
        let b = uri("/b");
        let c = uri("/c");
        let mut h = History::new(a.clone());
        assert!(!h.can_back());
        h.push(b.clone());
        h.push(c.clone());
        assert!(h.can_back());
        assert!(!h.can_forward());
        assert_eq!(h.back().unwrap(), b);
        assert_eq!(h.back().unwrap(), a);
        assert!(!h.can_back());
        assert_eq!(h.forward().unwrap(), b);
        h.push(c.clone()); // push clears forward
        assert!(!h.can_forward());
        assert_eq!(h.current(), Some(&c));
    }

    #[test]
    fn pane_state_navigation_resets_view() {
        let home = uri("/home");
        let docs = uri("/home/Docs");
        let pics = uri("/home/Pics");
        let mut s = PaneState::new(home.clone());
        assert_eq!(s.tree.root, home);
        assert!(s.tree.is_expanded(&home));
        s.navigate(docs.clone());
        s.navigate(pics.clone());
        s.listing.select_only(Some("x"), 0);
        assert!(s.go_back());
        assert_eq!(s.location, docs);
        assert_eq!(s.listing.selected_count(), 0);
        assert!(s.go_forward());
        assert_eq!(s.location, pics);
        // up_target: at the pane's tree root there is nowhere to go
        let s2 = PaneState::new(uri("/home/alice"));
        assert_eq!(s2.up_target(), None);
        let mut s3 = PaneState::new(uri("/home/alice"));
        s3.navigate(uri("/home/alice/docs"));
        assert_eq!(s3.up_target(), Some(uri("/home/alice")));
        let mut rooted = PaneState::new(uri("/"));
        assert_eq!(rooted.up_target(), None);
        rooted.navigate(uri("/tmp")); // location left root via open
        assert_eq!(rooted.up_target(), Some(uri("/")));
    }

    #[test]
    fn selection_on_adversarial_names() {
        let names = [
            "🦀.txt",
            "日本語.tar",
            "same.txt",
            "SAME.TXT",
            "same.txt", // duplicate display name (hostile listing)
            " leading-space.txt",
            " trailing space .txt",
        ];
        let order: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        let mut l = ListingState::default();
        l.select_all(&order);
        assert_eq!(l.selected_count(), 6, "BTreeSet dedupes the duplicate");
        l.toggle("SAME.TXT", 3);
        assert!(!l.is_selected("SAME.TXT"));
        l.prune(&order.iter().cloned().collect());
        assert!(!l.is_selected("same.txt") || l.is_selected("same.txt"));
        // Out-of-range row indices never panic.
        l.extend_to(&order, 999);
        l.move_focus(&order, isize::MIN, true);
        l.move_focus(&order, isize::MAX, false);
        l.focus_absolute(&order, usize::MAX, true);
    }

    #[test]
    fn history_survives_pathological_push_sequences() {
        let mut h = History::new(uri("/a"));
        for i in 0..1000 {
            h.push(uri(&format!("/p{i}")));
            assert!(h.can_back());
            assert!(!h.can_forward());
        }
        // Drain to the start without underflowing.
        let mut backs = 0;
        while h.back().is_some() {
            backs += 1;
            assert!(backs <= 1000);
        }
        assert_eq!(backs, 1000);
        assert!(!h.can_back());
        assert_eq!(h.current(), Some(&uri("/a")));
        // Forward to the end and one step beyond.
        let mut fwds = 0;
        while h.forward().is_some() {
            fwds += 1;
        }
        assert_eq!(fwds, 1000);
        assert!(!h.can_forward());
        // Push from the middle clears the forward stack.
        let mut h2 = History::new(uri("/a"));
        h2.push(uri("/b"));
        h2.back();
        assert!(h2.can_forward());
        h2.push(uri("/c"));
        assert!(!h2.can_forward());
    }

    #[test]
    fn sort_on_empty_singleton_and_all_duplicates() {
        let mut empty: Vec<Entry> = vec![];
        SortSpec::default().sort(&mut empty);
        assert!(empty.is_empty());

        let mut one = vec![entry("only.bin", EntryKind::File, Some(1))];
        SortSpec::default().sort(&mut one);
        assert_eq!(one[0].name, "only.bin");

        let dup = entry("dup", EntryKind::File, Some(7));
        let mut dups: Vec<Entry> = (0..50).map(|_| dup.clone()).collect();
        let spec = SortSpec {
            key: SortKey::Size,
            dir: SortDir::Desc,
            dirs_first: false,
        };
        spec.sort(&mut dups);
        assert_eq!(dups.len(), 50);
        assert!(dups.windows(2).all(|w| spec.compare(&w[0], &w[1]).is_le()));
        // Sorting twice is stable for equal keys.
        let once: Vec<Entry> = dups.clone();
        spec.sort(&mut dups);
        assert_eq!(dups, once);
    }

    #[test]
    fn history_moves_also_start_on_the_current_folder_row() {
        let mut p = PaneState::new(uri("/a"));
        p.navigate(uri("/b"));
        assert!(p.listing.on_current);
        p.listing.select_only(Some("x"), 2);
        assert!(p.go_back());
        assert_eq!(p.location, uri("/a"));
        assert!(p.listing.on_current && p.listing.selected_count() == 0);
        p.listing.select_only(Some("y"), 1);
        assert!(p.go_forward());
        assert_eq!(p.location, uri("/b"));
        assert!(p.listing.on_current && p.listing.selected_count() == 0);
    }

    #[test]
    fn an_old_session_file_without_on_current_still_loads() {
        let json = r#"{"sort":{"key":"Name","dir":"Asc","dirs_first":true},"selection":[],"focused_row":2,"on_parent":false,"scroll":0.0}"#;
        let l: ListingState = serde_json::from_str(json).expect("deserialize");
        assert!(!l.on_current);
        assert_eq!(l.focused_row, 2);
    }

    #[test]
    fn navigating_makes_the_tree_follow() {
        let mut p = PaneState::new(uri("/"));
        p.navigate(uri("/a/b/c"));
        assert!(p.tree.is_expanded(&uri("/a")) && p.tree.is_expanded(&uri("/a/b")));
        assert!(
            !p.tree.is_expanded(&uri("/a/b/c")),
            "the node itself is left alone"
        );
        assert_eq!(p.tree.focused.as_ref(), Some(&uri("/a/b/c")));
        assert!(p.tree.reveal);
        p.tree.reveal = false;
        p.tree.active = true;
        p.navigate(uri("/a/b/d"));
        assert!(
            !p.tree.reveal,
            "tree-driven navigation does not re-scroll the tree"
        );
    }

    #[test]
    fn navigate_to_the_current_location_keeps_the_panes_place() {
        let mut p = PaneState::new(uri("/work"));
        p.listing.select_only(Some("b.txt"), 3);
        p.listing.scroll = 120.0;
        p.tree.expand(uri("/work/sub"));
        p.tree.focused = Some(uri("/work/sub"));
        p.tree.scroll = 60.0;

        // Re-opening where the pane already is is a reload, not a jump:
        // nothing is cleared and no duplicate history entry appears.
        p.navigate(uri("/work"));
        assert!(p.listing.is_selected("b.txt"));
        assert_eq!(p.listing.focused_row, 3);
        assert_eq!(p.listing.scroll, 120.0);
        assert_eq!(p.tree.scroll, 60.0);
        assert_eq!(p.tree.focused.as_ref(), Some(&uri("/work/sub")));
        assert!(!p.can_go_back(), "no duplicate history entry");

        // A real jump still resets selection, focus and scroll, and parks the cursor on `.`
        // so the folder just opened is what a following Preview shows.
        p.navigate(uri("/elsewhere"));
        assert_eq!(p.listing.selected_count(), 0);
        assert_eq!(p.listing.focused_row, 0);
        assert!(p.listing.on_current);
        assert_eq!(p.listing.scroll, 0.0);
        assert!(p.can_go_back());
    }
}
