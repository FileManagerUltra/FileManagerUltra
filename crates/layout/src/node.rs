//! The pane tree: leaves are panes, nodes are two-way splits.

use crate::PaneId;

/// Split direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// The pane tree.
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

pub const MIN_RATIO: f32 = 0.05;
pub const MAX_RATIO: f32 = 0.95;

pub(crate) fn clamp_ratio(ratio: f32) -> f32 {
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

    /// Set a new ratio on the split whose handle is `handle`; false if absent.
    pub fn set_ratio(&mut self, handle: PaneId, ratio: f32) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split { a, b, ratio: r, .. } => {
                // The handle is identified by the left/top leaf id of the pair.
                let leftmost = a.panes().first().copied();
                if leftmost == Some(handle) {
                    *r = clamp_ratio(ratio);
                    true
                } else {
                    a.set_ratio(handle, ratio) || b.set_ratio(handle, ratio)
                }
            }
        }
    }

    pub(crate) fn split_leaf(
        &mut self,
        target: PaneId,
        axis: Axis,
        ratio: f32,
        new_id: PaneId,
    ) -> bool {
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
    pub(crate) fn close_leaf(&mut self, target: PaneId) -> bool {
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

    pub(crate) fn swap_leaves(&mut self, x: PaneId, y: PaneId) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_ratio_via_leftmost_leaf_handle() {
        let mut n = PaneNode::Split {
            axis: Axis::Horizontal,
            ratio: 0.5,
            a: Box::new(PaneNode::Leaf(1)),
            b: Box::new(PaneNode::Leaf(2)),
        };
        assert!(n.set_ratio(1, 0.8));
        assert!(matches!(&n, PaneNode::Split { ratio, .. } if (*ratio - 0.8).abs() < f32::EPSILON));
        assert!(!n.set_ratio(99, 0.1));
        // NaN falls back to 0.5
        assert!(n.set_ratio(1, f32::NAN));
        assert!(matches!(&n, PaneNode::Split { ratio, .. } if *ratio == 0.5));
    }
}
