use alloc::collections::VecDeque;

use crate::{Cell, row::Row};

/// A bounded row packer. Positions count emitted rows, including evicted history.
pub(crate) struct Reflow {
    pub rows: VecDeque<Row>,
    pub emitted: usize,
    pub first: usize,
    row: Row,
    col: u16,
    width: u16,
    limit: usize,
    retain_end: usize,
}

impl Reflow {
    pub fn new(width: u16, limit: usize) -> Self {
        Self {
            rows: VecDeque::new(),
            emitted: 0,
            first: 0,
            row: Row::new(width),
            col: 0,
            width,
            limit,
            retain_end: usize::MAX,
        }
    }

    pub fn position(&self) -> (usize, u16) {
        (self.emitted, self.col)
    }

    pub fn prepare(&mut self, width: u16, on_row: &mut impl FnMut(&Row)) {
        if self.col.saturating_add(width) > self.width {
            self.finish(true, on_row);
        }
    }

    // Resizing discards excess bottom rows, not the row containing the cursor.
    pub fn retain_through(&mut self, end: usize) {
        self.retain_end = end;
    }

    pub fn push(&mut self, cell: Cell, continuation: Option<Cell>) {
        if let Some(target) = self.row.get_mut(self.col) {
            *target = cell;
        }
        self.col += 1;
        if let Some(continuation) = continuation {
            if let Some(target) = self.row.get_mut(self.col) {
                *target = continuation;
            }
            self.col += 1;
        }
    }

    pub fn finish(&mut self, wrapped: bool, on_row: &mut impl FnMut(&Row)) {
        self.row.wrap(wrapped);
        if self.emitted < self.retain_end {
            if self.rows.len() == self.limit
                && let Some(row) = self.rows.pop_front()
            {
                self.first += 1;
                on_row(&row);
            }
            self.rows.push_back(core::mem::replace(
                &mut self.row,
                Row::new(self.width),
            ));
        } else {
            self.row = Row::new(self.width);
        }
        self.emitted += 1;
        self.col = 0;
    }
}
