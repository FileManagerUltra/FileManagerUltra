//! Range selection shared by every list that selects rows the way the Files listing
//! does: click, Ctrl-click, Shift-click and Shift+arrows with a fixed anchor. Generic
//! over the key, so the listing selects names and the Media playlist queue indices.

use std::collections::BTreeSet;

/// A selection and its focused row in a display `order` of keys.
#[derive(Debug)]
pub struct RangeSelection<'a, K: Ord + Clone> {
    pub selection: &'a mut BTreeSet<K>,
    pub focused_row: &'a mut usize,
}

impl<K: Ord + Clone> RangeSelection<'_, K> {
    /// Clear the selection and select `key` (if any), focusing `row`.
    pub fn select_only(&mut self, key: Option<&K>, row: usize) {
        self.selection.clear();
        if let Some(key) = key {
            self.selection.insert(key.clone());
        }
        *self.focused_row = row;
    }

    /// Ctrl-click: toggle `key` and focus its row.
    pub fn toggle(&mut self, key: &K, row: usize) {
        if !self.selection.remove(key) {
            self.selection.insert(key.clone());
        }
        *self.focused_row = row;
    }

    /// Select the range from the focused row to `row` (inclusive), replacing the selection.
    pub fn extend_to(&mut self, order: &[K], row: usize) {
        self.selection.clear();
        let Some(last) = order.len().checked_sub(1) else {
            *self.focused_row = 0;
            return;
        };
        let row = row.min(last);
        let anchor = (*self.focused_row).min(last);
        let (lo, hi) = (anchor.min(row), anchor.max(row));
        if let Some(keys) = order.get(lo..=hi) {
            self.selection.extend(keys.iter().cloned());
        }
        *self.focused_row = row;
    }

    /// Shift-click: grow the selection to `row`. Below the selection the range starts at its
    /// first row, above it at its last row, so earlier picks are kept; inside it shrinks to
    /// the range from its first row. With nothing selected it behaves like [`Self::extend_to`].
    pub fn extend_click(&mut self, order: &[K], row: usize) {
        let picked: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, key)| self.selection.contains(*key))
            .map(|(i, _)| i)
            .collect();
        let (Some(&lo), Some(&hi), Some(last)) =
            (picked.first(), picked.last(), order.len().checked_sub(1))
        else {
            self.extend_to(order, row);
            return;
        };
        let row = row.min(last);
        let (from, to) = if row < lo { (row, hi) } else { (lo, row) };
        self.selection.clear();
        if let Some(keys) = order.get(from..=to) {
            self.selection.extend(keys.iter().cloned());
        }
        *self.focused_row = row;
    }

    /// Select every key in `order` (Ctrl+A).
    pub fn select_all(&mut self, order: &[K]) {
        *self.selection = order.iter().cloned().collect();
    }

    /// The fixed end of a keyboard range: the selected end opposite the focused row (the
    /// focused row itself when the selection is empty or the focus is inside it).
    pub fn anchor_row(&self, order: &[K]) -> usize {
        let last = order.len().saturating_sub(1);
        let focus = (*self.focused_row).min(last);
        let mut picked = order
            .iter()
            .enumerate()
            .filter(|(_, key)| self.selection.contains(*key))
            .map(|(i, _)| i);
        let (Some(lo), hi) = (picked.next(), picked.next_back()) else {
            return focus;
        };
        let hi = hi.unwrap_or(lo);
        if focus == hi {
            lo
        } else if focus == lo {
            hi
        } else {
            focus
        }
    }

    /// Move the focus by `delta` rows; `extend` grows the range from the anchor, otherwise
    /// the selection collapses to the newly focused key.
    pub fn move_focus(&mut self, order: &[K], delta: isize, extend: bool) {
        let Some(last) = order.len().checked_sub(1) else {
            *self.focused_row = 0;
            self.selection.clear();
            return;
        };
        let current = (*self.focused_row).min(last) as isize;
        let next = current.saturating_add(delta).clamp(0, last as isize) as usize;
        self.focus_absolute(order, next, extend);
    }

    /// Jump to `row` (Home/End/Page), with the same semantics as [`Self::move_focus`].
    pub fn focus_absolute(&mut self, order: &[K], row: usize, extend: bool) {
        let Some(last) = order.len().checked_sub(1) else {
            *self.focused_row = 0;
            self.selection.clear();
            return;
        };
        let row = row.min(last);
        if extend {
            *self.focused_row = self.anchor_row(order);
            self.extend_to(order, row);
        } else {
            let key = order.get(row).cloned();
            self.select_only(key.as_ref(), row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sel<'a>(set: &'a mut BTreeSet<usize>, focus: &'a mut usize) -> RangeSelection<'a, usize> {
        RangeSelection {
            selection: set,
            focused_row: focus,
        }
    }

    #[test]
    fn shift_arrows_keep_the_anchor_and_flip_over_it() {
        let order: Vec<usize> = (10..20).collect();
        let (mut set, mut focus) = (BTreeSet::new(), 0);
        let mut s = sel(&mut set, &mut focus);
        s.select_only(Some(&13), 3);
        for _ in 0..3 {
            s.move_focus(&order, 1, true);
        }
        assert_eq!(*s.focused_row, 6);
        assert_eq!(
            s.selection.iter().copied().collect::<Vec<_>>(),
            [13, 14, 15, 16]
        );
        for _ in 0..5 {
            s.move_focus(&order, -1, true);
        }
        assert_eq!(
            s.selection.iter().copied().collect::<Vec<_>>(),
            [11, 12, 13]
        );
        s.move_focus(&order, 1, false);
        assert_eq!(s.selection.iter().copied().collect::<Vec<_>>(), [12]);
    }

    #[test]
    fn clicks_toggle_and_extend_like_the_listing() {
        let order: Vec<usize> = (0..6).collect();
        let (mut set, mut focus) = (BTreeSet::new(), 0);
        let mut s = sel(&mut set, &mut focus);
        s.select_only(Some(&1), 1);
        s.toggle(&4, 4);
        s.extend_click(&order, 5);
        assert_eq!(
            s.selection.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5]
        );
        s.extend_click(&order, 0);
        assert_eq!(s.selection.len(), 6);
        s.focus_absolute(&order, usize::MAX, true);
        assert_eq!(*s.focused_row, 5);
    }

    #[test]
    fn empty_and_hostile_inputs_never_panic() {
        let (mut set, mut focus) = (BTreeSet::new(), usize::MAX);
        let mut s = sel(&mut set, &mut focus);
        s.move_focus(&[], isize::MAX, true);
        s.extend_click(&[], 3);
        s.focus_absolute(&[], 9, false);
        assert!(s.selection.is_empty());
        let order = [1usize, 2];
        *s.focused_row = usize::MAX;
        s.move_focus(&order, isize::MIN, true);
        assert_eq!(s.selection.len(), 2);
        // Back down past the anchor: the range flips to just the last row.
        s.move_focus(&order, isize::MAX, true);
        assert_eq!(s.selection.iter().copied().collect::<Vec<_>>(), [2]);
    }
}
