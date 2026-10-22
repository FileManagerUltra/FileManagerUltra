//! Pane layout model: a recursive tree of splits and leaves, plus per-pane view state.
//! Pure data with structural invariants — no I/O, no egui.

use fm_vfs::{FmError, VfsUri};

/// Stable id of a pane (leaf). Never reused within a layout.
pub type PaneId = u64;

/// Split direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// The pane tree: leaves are panes, nodes are two-way splits.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneNode {
    Leaf(PaneId),
    Split {
        axis: Axis,
        /// Fraction of the space given to `a`, clamped to `MIN_RATIO..=MAX_RATIO`.
        ratio: f32,
        a: Box<PaneNode>,
        b: Box<PaneNode>,
    },
}

const MIN_RATIO: f32 = 0.05;
const MAX_RATIO: f32 = 0.95;

fn clamp_ratio(ratio: f32) -> f32 {
    // NaN and infinities clamp to the safe band.
    if !ratio.is_finite() {
        return 0.5;
    }
    ratio.clamp(MIN_RATIO, MAX_RATIO)
}

impl PaneNode {
    /// All leaf ids, in painting order.
    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_into(&mut out);
        out
    }

    fn collect_into(&self, out: &mut Vec<PaneId>) {
        match self {
            Self::Leaf(id) => out.push(*id),
            Self::Split { a, b, .. } => {
                a.collect_into(out);
                b.collect_into(out);
            }
        }
    }

    pub fn contains(&self, id: PaneId) -> bool {
        match self {
            Self::Leaf(me) => *me == id,
            Self::Split { a, b, .. } => a.contains(id) || b.contains(id),
        }
    }

    fn split_leaf(&mut self, target: PaneId, axis: Axis, ratio: f32, new_id: PaneId) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                *self = Self::Split {
                    axis,
                    ratio: clamp_ratio(ratio),
                    a: Box::new(Self::Leaf(target)),
                    b: Box::new(Self::Leaf(new_id)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { a, b, .. } => {
                a.split_leaf(target, axis, ratio, new_id)
                    || b.split_leaf(target, axis, ratio, new_id)
            }
        }
    }

    /// Replace the leaf `target` with its sibling; returns false if absent.
    fn close_leaf(&mut self, target: PaneId) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split { a, b, .. } => match (&**a, &**b) {
                (Self::Leaf(x), _) if *x == target => {
                    *self = (**b).clone();
                    true
                }
                (_, Self::Leaf(x)) if *x == target => {
                    *self = (**a).clone();
                    true
                }
                _ => a.close_leaf(target) || b.close_leaf(target),
            },
        }
    }

    fn swap_leaves(&mut self, x: PaneId, y: PaneId) -> bool {
        match self {
            Self::Leaf(id) if *id == x => {
                *id = y;
                true
            }
            Self::Leaf(id) if *id == y => {
                *id = x;
                true
            }
            Self::Leaf(_) => false,
            Self::Split { a, b, .. } => {
                // Both sides must run: x and y usually live in different subtrees.
                let found_a = a.swap_leaves(x, y);
                let found_b = b.swap_leaves(x, y);
                found_a || found_b
            }
        }
    }
}

/// View state of one pane: where it points, its history and the current selection.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneState {
    pub location: VfsUri,
    pub history: Vec<VfsUri>,
    /// Index into `history` of the current entry.
    pub history_index: usize,
    pub selected: Option<VfsUri>,
}

impl PaneState {
    pub fn new(location: VfsUri) -> Self {
        Self {
            history: vec![location.clone()],
            history_index: 0,
            location,
            selected: None,
        }
    }

    /// Navigate: truncate any forward history, push the new location.
    pub fn navigate(&mut self, to: VfsUri) {
        self.history.truncate(self.history_index + 1);
        self.history.push(to.clone());
        self.history_index = self.history.len() - 1;
        self.location = to;
        self.selected = None;
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    pub fn go_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.history_index -= 1;
        self.location = self.history[self.history_index].clone();
        self.selected = None;
        true
    }

    pub fn go_forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.history_index += 1;
        self.location = self.history[self.history_index].clone();
        self.selected = None;
        true
    }
}

/// The full pane tree plus per-pane state.
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PaneNode,
    pub states: std::collections::BTreeMap<PaneId, PaneState>,
    next_id: PaneId,
}

impl Layout {
    /// A single pane at `location`.
    pub fn single(location: VfsUri) -> Self {
        let mut states = std::collections::BTreeMap::new();
        states.insert(1, PaneState::new(location));
        Self {
            root: PaneNode::Leaf(1),
            states,
            next_id: 2,
        }
    }

    /// Split the pane `target`, returning the new pane's id. Invariants: ids are
    /// never reused, ratios are clamped, the target keeps its state.
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
        self.root.close_leaf(target);
        self.states.remove(&target);
        Ok(())
    }

    /// Swap the positions of two panes (ids follow the content).
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
