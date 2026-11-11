//! Pane layout model: a recursive tree of splits and leaves, plus per-pane view state.
//! Pure data with structural invariants — no I/O, no egui.

use std::collections::BTreeMap;

use fm_vfs::{FmError, VfsUri};

mod node;
pub mod selection;
mod state;

pub use node::{Axis, Edge, MAX_RATIO, MIN_RATIO, PaneNode};
pub use state::{
    DEFAULT_COL_WIDTHS, DEFAULT_TREE_WIDTH, DiffMode, DiffSide, DiffState, Focus, History,
    ListingState, MAX_COL_WIDTH, MIN_COL_WIDTH, MediaPaneState, PaneKind, PaneState, PlacesState,
    PreviewSource, SortDir, SortKey, SortSpec, TreeState,
};

/// Stable id of a pane (leaf). Never reused within a layout.
pub type PaneId = u64;

/// Most panes a layout may hold; larger restored layouts are rejected.
pub const MAX_PANES: usize = 64;

/// The full pane tree plus per-pane state and focus bookkeeping.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Layout {
    pub root: PaneNode,
    pub states: BTreeMap<PaneId, PaneState>,
    pub focus: Focus,
    /// When set, only this pane is shown (the others keep their state).
    #[serde(default)]
    pub zoomed: Option<PaneId>,
    next_id: PaneId,
}

impl Layout {
    /// A single pane at `location` whose tree is rooted at `root`.
    pub fn single_rooted(location: VfsUri, root: VfsUri, first_id: PaneId) -> Self {
        let mut l = Self::single(location.clone());
        l.states.insert(1, PaneState::new_rooted(location, root));
        l.relabel(first_id);
        l
    }

    /// A single pane at `location`.
    pub fn single(location: VfsUri) -> Self {
        let mut states = BTreeMap::new();
        states.insert(1, PaneState::new(location));
        Self {
            root: PaneNode::Leaf(1),
            states,
            focus: Focus {
                focused: 1,
                previous: None,
                last_files: Some(1),
            },
            zoomed: None,
            next_id: 2,
        }
    }

    /// A single pane whose id is `first_id`, so layouts that share one session never clash.
    pub fn single_at(location: VfsUri, first_id: PaneId) -> Self {
        let mut l = Self::single(location);
        l.relabel(first_id);
        l
    }

    /// Renumber the panes `first_id`, `first_id + 1`, … in painting order and return the
    /// old → new map. Per-pane engine state is keyed by id, so every layout of a session
    /// takes ids from its own range.
    pub fn relabel(&mut self, first_id: PaneId) -> BTreeMap<PaneId, PaneId> {
        let map: BTreeMap<PaneId, PaneId> = self
            .panes()
            .into_iter()
            .enumerate()
            .map(|(i, old)| (old, first_id.saturating_add(i as PaneId)))
            .collect();
        self.root.relabel(&map);
        let old_states = std::mem::take(&mut self.states);
        for (old, st) in old_states {
            if let Some(new) = map.get(&old) {
                self.states.insert(*new, st);
            }
        }
        for st in self.states.values_mut() {
            if let PreviewSource::Pane(p) = st.preview_source {
                st.preview_source = map
                    .get(&p)
                    .map_or(PreviewSource::Follow, |n| PreviewSource::Pane(*n));
            }
        }
        let remap = |p: PaneId| map.get(&p).copied();
        self.focus.focused = remap(self.focus.focused)
            .or_else(|| self.states.keys().next().copied())
            .unwrap_or(first_id);
        self.focus.previous = self.focus.previous.and_then(remap);
        self.focus.last_files = self.focus.last_files.and_then(remap);
        self.zoomed = self.zoomed.and_then(remap);
        self.next_id = first_id.saturating_add(map.len() as PaneId);
        map
    }

    /// Check a layout that came from outside (a saved workspace): the tree and the state
    /// map must describe the same panes, and soft values (ratios, widths, focus) are
    /// repaired. Never panics on hostile data.
    pub fn validate(&mut self) -> Result<(), FmError> {
        let panes = self.panes();
        let bad = |what: &str| FmError::InvalidParams(format!("saved layout: {what}"));
        if panes.is_empty() || panes.len() > MAX_PANES {
            return Err(bad("pane count out of range"));
        }
        let unique: std::collections::BTreeSet<PaneId> = panes.iter().copied().collect();
        if unique.len() != panes.len() {
            return Err(bad("duplicate pane"));
        }
        if self.states.len() != panes.len() || !panes.iter().all(|p| self.states.contains_key(p)) {
            return Err(bad("pane states do not match the tree"));
        }
        let f = |v: &mut f32| {
            if !v.is_finite() || *v < 0.0 {
                *v = 0.0;
            }
        };
        for st in self.states.values_mut() {
            f(&mut st.tree.scroll);
            f(&mut st.listing.scroll);
            if !st.tree_width.is_finite() {
                st.tree_width = DEFAULT_TREE_WIDTH;
            }
            if st.col_widths == state::LEGACY_COL_WIDTHS {
                st.col_widths = DEFAULT_COL_WIDTHS;
            }
            for ix in 0..3 {
                let w = st.col_widths.get(ix).copied().unwrap_or(0.0);
                st.set_col_width(ix, w);
                if !w.is_finite() {
                    st.reset_col_width(ix);
                }
            }
        }
        Ok(())
    }

    /// Split the pane `target`, returning the new pane's id. The new pane
    /// inherits a copy of the target's state and becomes focused.
    pub fn split(&mut self, target: PaneId, axis: Axis, ratio: f32) -> Result<PaneId, FmError> {
        if !self.root.contains(target) {
            return Err(FmError::NotFound(format!("pane {target}")));
        }
        let state = self
            .states
            .get(&target)
            .cloned()
            .ok_or_else(|| FmError::NotFound(format!("pane state {target}")))?;
        let new_id = self.next_id;
        self.next_id += 1;
        self.states.insert(new_id, state);
        self.root.split_leaf(target, axis, ratio, new_id);
        self.focus_new(new_id);
        self.fix_zoom();
        Ok(new_id)
    }

    /// Close `target`, promoting its sibling. The last pane cannot be closed.
    pub fn close(&mut self, target: PaneId) -> Result<(), FmError> {
        if matches!(self.root, PaneNode::Leaf(_)) {
            return Err(FmError::Unsupported("cannot close the last pane"));
        }
        if !self.root.contains(target) {
            return Err(FmError::NotFound(format!("pane {target}")));
        }
        let panes = self.panes();
        let survivors: Vec<PaneId> = panes.iter().copied().filter(|p| *p != target).collect();
        let old_focused = self.focus.focused;
        let old_previous = self.focus.previous;
        let focused_after = if old_focused != target && survivors.contains(&old_focused) {
            old_focused
        } else {
            old_previous
                .filter(|p| *p != target && survivors.contains(p))
                .or_else(|| survivors.first().copied())
                .ok_or(FmError::Unsupported("cannot close the last pane"))?
        };
        self.root.close_leaf(target);
        self.states.remove(&target);
        for st in self.states.values_mut() {
            if st.preview_source == PreviewSource::Pane(target) {
                st.preview_source = PreviewSource::Follow;
            }
        }
        if self.focus.last_files == Some(target) {
            self.focus.last_files = None;
        }
        self.focus.focused = focused_after;
        self.focus.previous = old_previous
            .filter(|p| *p != target && *p != focused_after && survivors.contains(p))
            .or_else(|| survivors.iter().copied().find(|p| *p != focused_after));
        self.fix_zoom();
        Ok(())
    }

    /// Swap the positions of two panes (ids — and therefore their state — follow).
    pub fn swap(&mut self, x: PaneId, y: PaneId) -> Result<(), FmError> {
        if x == y {
            return Ok(());
        }
        if !self.root.contains(x) || !self.root.contains(y) {
            return Err(FmError::NotFound("swap: unknown pane".to_string()));
        }
        self.root.swap_leaves(x, y);
        Ok(())
    }

    /// Re-insert `pane` beside `target` on `edge`; state and focus travel with the pane.
    /// Validates before touching the tree, so a failed move changes nothing.
    pub fn move_pane(&mut self, pane: PaneId, target: PaneId, edge: Edge) -> Result<(), FmError> {
        if pane == target {
            return Err(FmError::InvalidParams(
                "cannot move a pane onto itself".to_string(),
            ));
        }
        if !self.root.contains(pane) || !self.root.contains(target) {
            return Err(FmError::NotFound("move: unknown pane".to_string()));
        }
        if !self.root.close_leaf(pane) || !self.root.insert_beside(target, pane, edge) {
            return Err(FmError::Unsupported("pane could not be moved"));
        }
        self.focus_pane(pane)?;
        self.zoomed = None;
        Ok(())
    }

    /// Equal space for panes along every split's axis.
    pub fn equalize(&mut self) {
        self.root.equalize();
    }

    /// The equalised ratio of one split (by handle), for a double-click reset.
    pub fn equal_ratio_at(&self, a_first: PaneId, b_first: PaneId) -> Option<f32> {
        self.root.equal_ratio_at(a_first, b_first)
    }

    pub fn set_ratio_at(&mut self, a_first: PaneId, b_first: PaneId, ratio: f32) -> bool {
        self.root.set_ratio_at(a_first, b_first, ratio)
    }

    pub fn equal_ratio(&self, handle: PaneId) -> Option<f32> {
        self.root.equal_ratio(handle)
    }

    /// Flip the split holding `pane` between side-by-side and stacked. False when there is none.
    pub fn rotate(&mut self, pane: PaneId) -> bool {
        self.root.rotate_parent(pane)
    }

    /// Move the nearest divider around `pane` on `axis` by `delta` (positive = right/down).
    pub fn resize(&mut self, pane: PaneId, axis: Axis, delta: f32) -> bool {
        delta.is_finite() && self.root.nudge(pane, axis, delta)
    }

    /// Show only the focused pane, or restore the full layout.
    pub fn toggle_zoom(&mut self) {
        self.zoomed = match self.zoomed {
            Some(_) => None,
            None if self.panes().len() > 1 => Some(self.focus.focused),
            None => None,
        };
    }

    /// Remember the focused pane as the latest Files pane. Call once per frame/poll.
    pub fn touch_files_focus(&mut self) {
        let f = self.focus.focused;
        if self.states.get(&f).is_some_and(|s| s.kind.is_files()) {
            self.focus.last_files = Some(f);
        }
    }

    /// The Files pane a Preview pane shows, or `None` when the layout has no Files pane.
    pub fn preview_target(&self, preview: PaneId) -> Option<PaneId> {
        let is_files =
            |id: &PaneId| *id != preview && self.states.get(id).is_some_and(|s| s.kind.is_files());
        let pinned = match self.states.get(&preview).map(|s| s.preview_source) {
            Some(PreviewSource::Pane(id)) => Some(id),
            _ => None,
        };
        pinned
            .filter(is_files)
            .or_else(|| self.focus.last_files.filter(is_files))
            .or_else(|| self.panes().into_iter().find(is_files))
    }

    fn fix_zoom(&mut self) {
        if self
            .zoomed
            .is_some_and(|z| z != self.focus.focused || !self.root.contains(z))
        {
            self.zoomed = None;
        }
    }

    /// Update a split's ratio via its handle (the left/top leaf id). No I/O.
    pub fn set_ratio(&mut self, handle: PaneId, ratio: f32) -> bool {
        self.root.set_ratio(handle, ratio)
    }

    /// Leaf ids in painting order.
    pub fn panes(&self) -> Vec<PaneId> {
        self.root.panes()
    }

    /// Keep the first Media pane in painting order and demote any restored duplicates.
    pub fn normalize_media_panes(&mut self) -> bool {
        let mut found = false;
        let mut changed = false;
        for pane in self.panes() {
            let Some(state) = self.states.get_mut(&pane) else {
                continue;
            };
            if state.kind != PaneKind::Media {
                continue;
            }
            if found {
                state.kind = PaneKind::Files;
                changed = true;
            } else {
                found = true;
            }
        }
        changed
    }

    pub fn state(&self, id: PaneId) -> Option<&PaneState> {
        self.states.get(&id)
    }

    pub fn state_mut(&mut self, id: PaneId) -> Option<&mut PaneState> {
        self.states.get_mut(&id)
    }

    /// Move focus to `id` (stashing the old one as previous). Errors if unknown.
    pub fn focus_pane(&mut self, id: PaneId) -> Result<(), FmError> {
        if !self.states.contains_key(&id) {
            return Err(FmError::NotFound(format!("pane {id}")));
        }
        if self.focus.focused != id {
            self.focus.previous = Some(self.focus.focused);
            self.focus.focused = id;
        }
        self.fix_zoom();
        Ok(())
    }

    pub fn focused(&self) -> PaneId {
        self.focus.focused
    }

    /// Focus the next leaf in painting order, wrapping.
    pub fn focus_next(&mut self) {
        let panes = self.panes();
        if panes.is_empty() {
            return;
        }
        let pos = panes
            .iter()
            .position(|p| *p == self.focus.focused)
            .unwrap_or(0);
        let next = panes[(pos + 1) % panes.len()];
        // focus_next cycles without stashing "previous" as the swap partner —
        // Tab-swap uses `swap_focused_with_previous`, not this.
        self.focus.focused = next;
        self.fix_zoom();
    }

    /// Commander-style Tab: swap the focused pane with the previously focused
    /// one; focus lands on the swap partner (the pane that was focused before).
    pub fn swap_focused_with_previous(&mut self) -> Result<(), FmError> {
        let prev = self
            .focus
            .previous
            .ok_or(FmError::Unsupported("no previous pane to swap with"))?;
        let cur = self.focus.focused;
        self.swap(cur, prev)?;
        // After the exchange, follow the pane that used to be `prev`.
        self.focus.focused = prev;
        self.focus.previous = Some(cur);
        self.fix_zoom();
        Ok(())
    }

    fn focus_new(&mut self, id: PaneId) {
        self.focus.previous = Some(self.focus.focused);
        self.focus.focused = id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout::single(VfsUri::parse("/home/alice").expect("static uri"))
    }

    #[test]
    fn single_pane_is_leaf() {
        let l = layout();
        assert_eq!(l.panes(), vec![1]);
        assert!(l.state(1).is_some());
        assert_eq!(l.focused(), 1);
    }

    #[test]
    fn split_adds_a_pane_and_keeps_ids_stable() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.3).expect("split");
        assert_eq!(l.panes(), vec![1, b]);
        assert_eq!(l.panes(), vec![1, 2]);
        let c = l.split(1, Axis::Horizontal, 0.5).expect("split");
        assert_eq!(l.panes(), vec![1, c, b]);
        assert_ne!(b, c);
        // ratios are clamped
        if let PaneNode::Split { ratio, .. } = &l.root {
            assert!((ratio - 0.3).abs() < f32::EPSILON);
        } else {
            panic!("expected split");
        }
        assert_eq!(l.focused(), c, "new pane is focused");
    }

    #[test]
    fn media_normalization_keeps_first_pane_in_painting_order() {
        let mut l = layout();
        let second = l.split(1, Axis::Vertical, 0.5).expect("split");
        l.state_mut(1).expect("first").kind = PaneKind::Media;
        l.state_mut(second).expect("second").kind = PaneKind::Media;
        assert!(l.normalize_media_panes());
        assert_eq!(l.state(1).expect("first").kind, PaneKind::Media);
        assert_eq!(l.state(second).expect("second").kind, PaneKind::Files);
        assert!(!l.normalize_media_panes());
    }

    #[test]
    fn ratio_clamping_and_nan() {
        let mut l = layout();
        l.split(1, Axis::Horizontal, f32::NAN).expect("split");
        if let PaneNode::Split { ratio, .. } = &l.root {
            assert_eq!(*ratio, 0.5);
        } else {
            panic!("expected split");
        }
        let mut l3 = layout();
        l3.split(1, Axis::Horizontal, 7.0).expect("split");
        if let PaneNode::Split { ratio, .. } = &l3.root {
            assert_eq!(*ratio, MAX_RATIO);
        } else {
            panic!("expected split");
        }
        let mut l4 = layout();
        l4.split(1, Axis::Horizontal, -1.0).expect("split");
        if let PaneNode::Split { ratio, .. } = &l4.root {
            assert_eq!(*ratio, MIN_RATIO);
        } else {
            panic!("expected split");
        }
    }

    #[test]
    fn close_promotes_sibling_and_last_is_undeletable() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        l.close(b).expect("close");
        assert_eq!(l.panes(), vec![1]);
        assert!(l.state(b).is_none());
        assert_eq!(l.focused(), 1, "focus moves to the survivor");
        let err = l.close(1).expect_err("close last");
        assert!(err.to_string().contains("last pane"));
    }

    #[test]
    fn closing_new_split_returns_focus_to_previously_active_pane() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        let c = l.split(b, Axis::Horizontal, 0.5).expect("split");

        assert_eq!(l.focused(), c);
        assert_eq!(l.focus.previous, Some(b));
        l.close(c).expect("close");

        assert_eq!(l.focused(), b, "restore the pane active before the split");
        assert_ne!(l.focus.previous, Some(c));
        assert!(l.focus.previous.is_none_or(|p| l.state(p).is_some()));
    }

    #[test]
    fn close_nested() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        let c = l.split(1, Axis::Horizontal, 0.5).expect("split");
        l.close(c).expect("close");
        assert_eq!(l.panes(), vec![1, b]);
    }

    #[test]
    fn swap_preserves_pane_set() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        let before: Vec<_> = l.panes();
        l.swap(1, b).expect("swap");
        assert_eq!(l.panes(), vec![b, 1]);
        let mut sorted = l.panes();
        sorted.sort();
        let mut expected = before.clone();
        expected.sort();
        assert_eq!(sorted, expected);
        l.swap(99, 1).expect_err("unknown pane");
    }

    #[test]
    fn split_unknown_pane_errors() {
        let mut l = layout();
        assert!(l.split(42, Axis::Vertical, 0.5).is_err());
    }

    #[test]
    fn split_preserves_sibling_state_and_moves_it() {
        let mut l = layout();
        {
            let s = l.state_mut(1).expect("state");
            s.tree
                .expand(VfsUri::parse("/home/alice/docs").expect("uri"));
            s.listing.sort.header_click(SortKey::Size);
            s.listing.select_only(Some("file.txt"), 3);
        }
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        // The original pane keeps its view state.
        let s1 = l.state(1).expect("state");
        assert!(
            s1.tree
                .is_expanded(&VfsUri::parse("/home/alice/docs").expect("uri"))
        );
        assert_eq!(s1.listing.sort.key, SortKey::Size);
        assert!(s1.listing.is_selected("file.txt"));
        assert_eq!(s1.listing.focused_row, 3);
        // The new pane inherits a copy.
        let sb = l.state(b).expect("state");
        assert_eq!(sb.listing.sort.key, SortKey::Size);
    }

    #[test]
    fn focus_tracking_and_next_cycles() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        assert_eq!(l.focused(), b);
        let c = l.split(1, Axis::Horizontal, 0.5).expect("split");
        assert_eq!(l.focused(), c);
        l.focus_next();
        assert_eq!(l.focused(), b, "next leaf in painting order");
        l.focus_next();
        assert_eq!(l.focused(), 1, "wraps to the first leaf");
        l.focus_pane(b).expect("focus");
        assert_eq!(l.focused(), b);
        l.focus_pane(99).expect_err("unknown");
    }

    #[test]
    fn tab_swaps_two_most_recent_panes() {
        let mut l = layout();
        let b = l.split(1, Axis::Vertical, 0.5).expect("split");
        // focused b (new), previous 1
        l.swap_focused_with_previous().expect("tab");
        assert_eq!(l.focused(), 1, "focus lands on the swap partner");
        assert_eq!(l.panes(), vec![b, 1], "positions exchanged");
        // state followed the ids
        assert_eq!(l.focus.previous, Some(b));
        l.swap_focused_with_previous().expect("tab back");
        assert_eq!(l.focused(), b);
        assert_eq!(l.panes(), vec![1, b]);
        // no previous on a fresh single pane
        let mut solo = layout();
        solo.swap_focused_with_previous().expect_err("no previous");
    }

    #[test]
    fn nested_splits_with_the_same_first_leaf_have_separate_handles() {
        // (A | B) over C: both splits start at A.
        let mut l = Layout::single(VfsUri::root_file());
        let a = l.focused();
        let c = l.split(a, Axis::Vertical, 0.5).expect("split");
        let b = l.split(a, Axis::Horizontal, 0.5).expect("split");
        let PaneNode::Split { ratio: outer, .. } = l.root else {
            panic!("root is a split");
        };
        assert!(l.set_ratio_at(a, b, 0.8));
        let PaneNode::Split { ratio, a: top, .. } = &l.root else {
            panic!("root is a split");
        };
        assert_eq!(*ratio, outer, "the outer split is untouched");
        assert!(matches!(&**top, PaneNode::Split { ratio, .. } if (*ratio - 0.8).abs() < 1e-6));
        assert!(l.set_ratio_at(a, c, 0.3));
        assert!(matches!(&l.root, PaneNode::Split { ratio, .. } if (*ratio - 0.3).abs() < 1e-6));
    }

    #[test]
    fn set_ratio_through_layout() {
        let mut l = layout();
        l.split(1, Axis::Horizontal, 0.5).expect("split");
        assert!(l.set_ratio(1, 0.7));
        if let PaneNode::Split { ratio, .. } = &l.root {
            assert!((ratio - 0.7).abs() < f32::EPSILON);
        } else {
            panic!("expected split");
        }
    }

    #[test]
    fn pane_state_history() {
        let home = VfsUri::parse("/home").expect("uri");
        let docs = VfsUri::parse("/home/Docs").expect("uri");
        let pics = VfsUri::parse("/home/Pics").expect("uri");
        let mut s = PaneState::new(home.clone());
        assert!(!s.can_go_back());
        s.navigate(docs.clone());
        s.navigate(pics.clone());
        assert!(s.go_back());
        assert_eq!(s.location, docs);
        assert!(s.go_forward());
        assert_eq!(s.location, pics);
        assert!(!s.go_forward());
        // navigating from the middle truncates forward history
        s.go_back();
        s.navigate(home.clone());
        assert!(!s.can_go_forward());
    }

    fn three_in_a_row() -> (Layout, PaneId, PaneId, PaneId) {
        let mut l = layout();
        let b = l.split(1, Axis::Horizontal, 0.5).expect("split");
        let c = l.split(b, Axis::Horizontal, 0.5).expect("split");
        (l, 1, b, c)
    }

    fn invariants(l: &Layout) {
        let mut leaves = l.panes();
        leaves.sort_unstable();
        let keys: Vec<PaneId> = l.states.keys().copied().collect();
        assert_eq!(leaves, keys, "leaf set equals state set");
        fn ratios(n: &PaneNode) {
            if let PaneNode::Split { ratio, a, b, .. } = n {
                assert!((MIN_RATIO..=MAX_RATIO).contains(ratio), "ratio {ratio}");
                ratios(a);
                ratios(b);
            }
        }
        ratios(&l.root);
        assert!(l.root.contains(l.focus.focused));
        if let Some(p) = l.focus.previous {
            assert!(l.states.contains_key(&p));
        }
        if let Some(z) = l.zoomed {
            assert_eq!(z, l.focus.focused);
        }
    }

    #[test]
    fn move_to_an_edge_reinserts_and_keeps_state() {
        let (mut l, a, b, c) = three_in_a_row();
        l.state_mut(a).expect("state").listing.sort.key = SortKey::Size;
        l.move_pane(a, c, Edge::Bottom).expect("move");
        invariants(&l);
        assert_eq!(l.panes(), vec![b, c, a]);
        assert_eq!(l.state(a).expect("state").listing.sort.key, SortKey::Size);
        assert_eq!(l.focused(), a);
        let PaneNode::Split {
            axis, b: second, ..
        } = &l.root
        else {
            panic!("root must be a split");
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert!(matches!(
            &**second,
            PaneNode::Split {
                axis: Axis::Vertical,
                ..
            }
        ));
    }

    #[test]
    fn move_left_and_top_take_the_first_slot() {
        let (mut l, a, b, _c) = three_in_a_row();
        l.move_pane(b, a, Edge::Top).expect("move");
        invariants(&l);
        assert_eq!(l.panes()[..2], [b, a]);
    }

    #[test]
    fn invalid_moves_change_nothing() {
        let (mut l, a, _b, _c) = three_in_a_row();
        let before = l.root.clone();
        assert!(l.move_pane(a, a, Edge::Left).is_err());
        assert!(l.move_pane(a, 99, Edge::Left).is_err());
        assert!(l.move_pane(99, a, Edge::Left).is_err());
        assert_eq!(l.root, before);
        let mut single = layout();
        assert!(single.move_pane(1, 2, Edge::Left).is_err());
    }

    #[test]
    fn equalize_weights_by_leaves_along_the_axis() {
        let (mut l, a, _b, _c) = three_in_a_row();
        l.set_ratio(a, 0.8);
        l.equalize();
        let PaneNode::Split { ratio, b, .. } = &l.root else {
            panic!("split");
        };
        assert!((ratio - 1.0 / 3.0).abs() < 1e-5, "{ratio}");
        let PaneNode::Split { ratio: inner, .. } = &**b else {
            panic!("split");
        };
        assert!((inner - 0.5).abs() < 1e-5);
        assert_eq!(l.equal_ratio(a), Some(*ratio));
    }

    #[test]
    fn rotate_flips_the_parent_axis_only() {
        let (mut l, _a, b, _c) = three_in_a_row();
        assert!(l.rotate(b));
        let PaneNode::Split { axis, b: inner, .. } = &l.root else {
            panic!("split");
        };
        assert_eq!(*axis, Axis::Horizontal);
        assert!(matches!(
            &**inner,
            PaneNode::Split {
                axis: Axis::Vertical,
                ..
            }
        ));
        assert!(!layout().rotate(1));
    }

    #[test]
    fn resize_moves_the_nearest_divider_on_that_axis() {
        let (mut l, a, _b, c) = three_in_a_row();
        assert!(l.resize(c, Axis::Horizontal, -0.1));
        let PaneNode::Split { ratio, b, .. } = &l.root else {
            panic!("split");
        };
        assert!((ratio - 0.5).abs() < 1e-5, "outer divider untouched");
        assert!(matches!(&**b, PaneNode::Split { ratio: r, .. } if (r - 0.4).abs() < 1e-5));
        // a has no vertical divider; NaN and clamping are safe
        assert!(!l.resize(a, Axis::Vertical, 0.1));
        assert!(!l.resize(a, Axis::Horizontal, f32::NAN));
        assert!(l.resize(a, Axis::Horizontal, 10.0));
        invariants(&l);
    }

    #[test]
    fn zoom_toggles_and_ends_when_focus_leaves() {
        let (mut l, a, b, _c) = three_in_a_row();
        l.focus_pane(b).expect("focus");
        l.toggle_zoom();
        assert_eq!(l.zoomed, Some(b));
        l.toggle_zoom();
        assert_eq!(l.zoomed, None);
        l.toggle_zoom();
        l.focus_pane(a).expect("focus");
        assert_eq!(l.zoomed, None);
        l.toggle_zoom();
        l.close(a).expect("close");
        assert_eq!(l.zoomed, None);
        assert!(layout().zoomed.is_none());
        let mut one = layout();
        one.toggle_zoom();
        assert_eq!(one.zoomed, None, "nothing to zoom with a single pane");
    }

    #[test]
    fn random_operations_keep_the_invariants() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let mut l = layout();
        for _ in 0..3000 {
            let panes = l.panes();
            let p = panes[next(panes.len())];
            let q = panes[next(panes.len())];
            let axis = if next(2) == 0 {
                Axis::Horizontal
            } else {
                Axis::Vertical
            };
            match next(9) {
                0 if panes.len() < 8 => {
                    let _ = l.split(p, axis, 0.5);
                }
                1 => {
                    let _ = l.close(p);
                }
                2 => {
                    let _ = l.swap(p, q);
                }
                3 => {
                    let _ = l.move_pane(p, q, Edge::ALL[next(4)]);
                }
                4 => l.equalize(),
                5 => {
                    l.rotate(p);
                }
                6 => {
                    l.resize(p, axis, (next(21) as f32 - 10.0) / 20.0);
                }
                7 => {
                    let _ = l.focus_pane(q);
                }
                _ => l.toggle_zoom(),
            }
            invariants(&l);
        }
    }

    #[test]
    fn preview_follows_the_last_focused_files_pane() {
        let mut l = Layout::single(VfsUri::root_file());
        let b = l.split(1, Axis::Horizontal, 0.5).expect("split");
        let p = l.split(b, Axis::Horizontal, 0.5).expect("split");
        l.state_mut(p).expect("state").kind = PaneKind::Preview;
        l.touch_files_focus();
        assert_eq!(
            l.preview_target(p),
            Some(1),
            "touch happened on the preview itself"
        );
        l.focus_pane(b).expect("focus");
        l.touch_files_focus();
        l.focus_pane(p).expect("focus");
        l.touch_files_focus();
        assert_eq!(
            l.preview_target(p),
            Some(b),
            "focusing the preview keeps the last Files pane"
        );
    }

    #[test]
    fn pinned_source_falls_back_when_closed_or_not_files() {
        let mut l = Layout::single(VfsUri::root_file());
        let b = l.split(1, Axis::Horizontal, 0.5).expect("split");
        let p = l.split(b, Axis::Horizontal, 0.5).expect("split");
        l.state_mut(p).expect("state").kind = PaneKind::Preview;
        l.state_mut(p).expect("state").preview_source = PreviewSource::Pane(1);
        assert_eq!(l.preview_target(p), Some(1));
        l.state_mut(1).expect("state").kind = PaneKind::Terminal;
        assert_eq!(
            l.preview_target(p),
            Some(b),
            "a pinned non-Files pane is ignored"
        );
        l.state_mut(1).expect("state").kind = PaneKind::Files;
        l.close(1).expect("close");
        assert_eq!(
            l.state(p).expect("state").preview_source,
            PreviewSource::Follow
        );
        assert_eq!(l.preview_target(p), Some(b));
    }

    #[test]
    fn preview_without_files_pane_has_no_target() {
        let mut l = Layout::single(VfsUri::root_file());
        l.state_mut(1).expect("state").kind = PaneKind::Preview;
        assert_eq!(l.preview_target(1), None);
    }

    #[test]
    fn relabel_moves_every_id_and_keeps_pins() {
        let mut l = Layout::single(VfsUri::root_file());
        let b = l.split(1, Axis::Horizontal, 0.5).expect("split");
        let c = l.split(b, Axis::Vertical, 0.5).expect("split");
        l.state_mut(c).expect("state").preview_source = PreviewSource::Pane(1);
        let map = l.relabel(1000);
        assert_eq!(l.panes(), vec![1000, 1001, 1002]);
        assert_eq!(map.get(&1), Some(&1000));
        assert_eq!(
            l.state(1002).expect("state").preview_source,
            PreviewSource::Pane(1000)
        );
        assert!(l.states.keys().all(|k| *k >= 1000));
        let d = l.split(1000, Axis::Vertical, 0.5).expect("split");
        assert_eq!(d, 1003, "new ids continue after the range");
        let single = Layout::single_at(VfsUri::root_file(), 5000);
        assert_eq!(single.panes(), vec![5000]);
        assert_eq!(single.focused(), 5000);
    }

    #[test]
    fn serde_round_trip_and_validate_reject_hostile_layouts() {
        let mut l = Layout::single(VfsUri::root_file());
        l.split(1, Axis::Horizontal, 0.3).expect("split");
        let json = serde_json::to_string(&l).expect("ser");
        let mut back: Layout = serde_json::from_str(&json).expect("de");
        back.validate().expect("valid");
        back.focus.focused = 99;
        back.relabel(1);
        assert!(
            back.state(back.focused()).is_some(),
            "focus repaired by relabel"
        );
        assert_eq!(back.panes(), l.panes());

        let mut dup = back.clone();
        dup.root = PaneNode::Split {
            axis: Axis::Horizontal,
            ratio: f32::NAN,
            a: Box::new(PaneNode::Leaf(1)),
            b: Box::new(PaneNode::Leaf(1)),
        };
        assert!(dup.validate().is_err(), "duplicate leaf");

        let mut orphan = back.clone();
        orphan.states.remove(&2);
        assert!(orphan.validate().is_err(), "leaf without state");

        let mut nan = back.clone();
        if let Some(st) = nan.state_mut(1) {
            st.tree_width = f32::NAN;
            st.listing.scroll = f32::INFINITY;
        }
        nan.validate().expect("soft values are repaired");
        let st = nan.state(1).expect("state");
        assert!(st.tree_width.is_finite() && st.listing.scroll == 0.0);
    }
}
