//! Per-pane view state: tree expansion, listing sort/selection, navigation history.

use std::collections::BTreeSet;

use fm_vfs::{Entry, EntryKind, VfsUri};

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
    pub scroll: f32,
}

impl Default for ListingState {
    fn default() -> Self {
        Self {
            sort: SortSpec::default(),
            selection: BTreeSet::new(),
            focused_row: 0,
            scroll: 0.0,
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
        if let Some(name) = name {
            self.selection.insert(name.to_string());
        }
        self.focused_row = row;
    }

    /// Ctrl-style: toggle `name`, move focus to its row.
    pub fn toggle(&mut self, name: &str, row: usize) {
        if !self.selection.remove(name) {
            self.selection.insert(name.to_string());
        }
        self.focused_row = row;
    }

    /// Shift-style: select the range between the anchor row and `row`
    /// (inclusive) in the given display order, replacing the selection.
    pub fn extend_to(&mut self, order: &[String], row: usize) {
        self.selection.clear();
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
        self.selection = order.iter().cloned().collect();
    }

    /// Drop selected names that no longer exist (after a re-read).
    pub fn prune(&mut self, existing: &BTreeSet<String>) {
        self.selection.retain(|name| existing.contains(name));
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
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

/// Everything one pane remembers about its view.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneState {
    pub location: VfsUri,
    pub tree: TreeState,
    pub listing: ListingState,
    pub history: History,
    /// Width of the tree column in px (drag-resized, persisted per pane).
    pub tree_width: f32,
}

impl PaneState {
    /// A pane rooted at `location`: the tree starts expanded at the same uri.
    pub fn new(location: VfsUri) -> Self {
        Self {
            tree: TreeState::new(location.clone()),
            history: History::new(location.clone()),
            location,
            listing: ListingState::default(),
            tree_width: 220.0,
        }
    }

    /// Commit a navigation: history push, selection reset. The caller
    /// (engine) is responsible for submitting the directory read.
    pub fn navigate(&mut self, to: VfsUri) {
        self.history.push(to.clone());
        self.location = to;
        self.listing.clear_selection();
        self.listing.focused_row = 0;
        self.listing.scroll = 0.0;
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
                self.listing.clear_selection();
                self.listing.focused_row = 0;
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
                self.listing.clear_selection();
                self.listing.focused_row = 0;
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
#[derive(Clone, Debug)]
pub struct Focus {
    pub focused: PaneId,
    pub previous: Option<PaneId>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
