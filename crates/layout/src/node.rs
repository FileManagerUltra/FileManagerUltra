//! The pane tree: leaves are panes, nodes are two-way splits.

use crate::PaneId;

/// Split direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// Which side of a target pane a moved pane is inserted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == s)
    }

    /// The split axis the edge creates.
    pub fn axis(self) -> Axis {
        match self {
            Self::Left | Self::Right => Axis::Horizontal,
            Self::Top | Self::Bottom => Axis::Vertical,
        }
    }

    /// Whether the moved pane takes the first (left/top) slot.
    pub fn first(self) -> bool {
        matches!(self, Self::Left | Self::Top)
    }
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

    /// Leaves counted along `axis`: same-axis children add up, cross-axis ones take the max.
    fn span(&self, axis: Axis) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split {
                axis: a,
                a: x,
                b: y,
                ..
            } if *a == axis => x.span(axis) + y.span(axis),
            Self::Split { a: x, b: y, .. } => x.span(axis).max(y.span(axis)),
        }
    }

    /// Give panes along each split's axis equal space.
    pub(crate) fn equalize(&mut self) {
        if let Self::Split { axis, ratio, a, b } = self {
            a.equalize();
            b.equalize();
            let (sa, sb) = (a.span(*axis), b.span(*axis));
            *ratio = clamp_ratio(sa as f32 / (sa + sb).max(1) as f32);
        }
    }

    /// Equalised ratio of the split whose handle (left/top leaf) is `handle`.
    pub fn equal_ratio(&self, handle: PaneId) -> Option<f32> {
        match self {
            Self::Leaf(_) => None,
            Self::Split { axis, a, b, .. } => {
                if a.panes().first() == Some(&handle) {
                    let (sa, sb) = (a.span(*axis), b.span(*axis));
                    Some(clamp_ratio(sa as f32 / (sa + sb).max(1) as f32))
                } else {
                    a.equal_ratio(handle).or_else(|| b.equal_ratio(handle))
                }
            }
        }
    }

    /// Flip the axis of the split that directly holds leaf `id`.
    pub(crate) fn rotate_parent(&mut self, id: PaneId) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split { axis, a, b, .. } => {
                if matches!(**a, Self::Leaf(x) if x == id)
                    || matches!(**b, Self::Leaf(x) if x == id)
                {
                    *axis = match axis {
                        Axis::Horizontal => Axis::Vertical,
                        Axis::Vertical => Axis::Horizontal,
                    };
                    true
                } else {
                    a.rotate_parent(id) || b.rotate_parent(id)
                }
            }
        }
    }

    /// Move the nearest divider above leaf `id` on `axis` by `delta` (positive = right/down).
    pub(crate) fn nudge(&mut self, id: PaneId, axis: Axis, delta: f32) -> bool {
        let Self::Split {
            axis: my,
            ratio,
            a,
            b,
        } = self
        else {
            return false;
        };
        let inner = if a.contains(id) {
            a.nudge(id, axis, delta)
        } else if b.contains(id) {
            b.nudge(id, axis, delta)
        } else {
            return false;
        };
        if inner {
            return true;
        }
        if *my == axis {
            *ratio = clamp_ratio(*ratio + delta);
            return true;
        }
        false
    }

    /// Replace leaf `target` with a split holding it and `moved` on `edge`.
    pub(crate) fn insert_beside(&mut self, target: PaneId, moved: PaneId, edge: Edge) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                let (first, second) = if edge.first() {
                    (moved, target)
                } else {
                    (target, moved)
                };
                *self = Self::Split {
                    axis: edge.axis(),
                    ratio: 0.5,
                    a: Box::new(Self::Leaf(first)),
                    b: Box::new(Self::Leaf(second)),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { a, b, .. } => {
                a.insert_beside(target, moved, edge) || b.insert_beside(target, moved, edge)
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
