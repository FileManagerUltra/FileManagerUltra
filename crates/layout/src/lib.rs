//! Pane layout model: a recursive tree of splits and leaves, plus per-pane view state.
//! Pure data with structural invariants — no I/O, no egui.

use std::collections::BTreeMap;

use fm_vfs::{FmError, VfsUri};

mod node;
mod state;

pub use node::{Axis, MAX_RATIO, MIN_RATIO, PaneNode};
pub use state::{Focus, History, ListingState, PaneState, SortDir, SortKey, SortSpec, TreeState};

/// Stable id of a pane (leaf). Never reused within a layout.
pub type PaneId = u64;

/// The full pane tree plus per-pane state and focus bookkeeping.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PaneNode,
    pub states: BTreeMap<PaneId, PaneState>,
    pub focus: Focus,
    next_id: PaneId,
}

impl Layout {
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
            },
            next_id: 2,
        }
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
        // Focus a surviving pane before the id disappears; never leave a
        // dangling previous-focus pointing at the closed id.
        let survivor = self.panes().into_iter().find(|p| *p != target);
        self.root.close_leaf(target);
        self.states.remove(&target);
        if let Some(s) = survivor {
            if self.focus.focused == target {
                self.focus.focused = s;
            }
            if self.focus.previous == Some(target) {
                self.focus.previous = Some(s);
            }
        }
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

    /// Update a split's ratio via its handle (the left/top leaf id). No I/O.
    pub fn set_ratio(&mut self, handle: PaneId, ratio: f32) -> bool {
        self.root.set_ratio(handle, ratio)
    }

    /// Leaf ids in painting order.
    pub fn panes(&self) -> Vec<PaneId> {
        self.root.panes()
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
}
