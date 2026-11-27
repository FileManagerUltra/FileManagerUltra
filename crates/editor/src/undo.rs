//! Undo/redo as a log of edit groups, bounded by a user-set step count and a fixed text
//! budget. Typing runs and backspace runs merge into one step.

use std::collections::VecDeque;

use crate::buffer::{Buffer, EditError, EditRecord};

pub const DEFAULT_MAX_STEPS: usize = 1000;
pub const MIN_STEPS: usize = 10;
pub const MAX_STEPS: usize = 100_000;
/// Stored text (removed + inserted) per file; the oldest groups drop beyond it.
pub const MAX_BYTES: usize = 64 << 20;

/// How an edit may merge with the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coalesce {
    /// Always a step of its own.
    No,
    /// Inserted text; merges while it continues the previous insertion.
    Typing,
    /// Deleted text (Backspace or Delete); merges while it continues the previous deletion.
    Deleting,
}

#[derive(Clone, Debug)]
struct Group {
    edits: Vec<EditRecord>,
    bytes: usize,
    kind: Coalesce,
}

fn edit_bytes(e: &EditRecord) -> usize {
    e.removed.len() + e.inserted.len()
}

#[derive(Clone, Debug)]
pub struct UndoLog {
    undo: VecDeque<Group>,
    redo: Vec<Group>,
    bytes: usize,
    max_steps: usize,
    max_bytes: usize,
    truncated: bool,
}

impl Default for UndoLog {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_STEPS)
    }
}

impl UndoLog {
    /// `max_steps` is clamped to `MIN_STEPS..=MAX_STEPS`.
    pub fn new(max_steps: usize) -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            bytes: 0,
            max_steps: max_steps.clamp(MIN_STEPS, MAX_STEPS),
            max_bytes: MAX_BYTES,
            truncated: false,
        }
    }

    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    /// Applies a new depth at once, dropping the oldest steps if it shrank.
    pub fn set_max_steps(&mut self, max_steps: usize) {
        self.max_steps = max_steps.clamp(MIN_STEPS, MAX_STEPS);
        self.trim();
    }

    #[cfg(test)]
    fn set_max_bytes(&mut self, max_bytes: usize) {
        self.max_bytes = max_bytes;
        self.trim();
    }

    pub fn undo_steps(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_steps(&self) -> usize {
        self.redo.len()
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// True once older history has been dropped to respect a limit.
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
        self.truncated = false;
    }

    /// Records an edit that was just applied to the buffer.
    pub fn record(&mut self, edit: EditRecord, kind: Coalesce) {
        self.redo.clear();
        let add = edit_bytes(&edit);
        if kind != Coalesce::No
            && let Some(last) = self.undo.back_mut()
            && last.kind == kind
            && continues(last.edits.last(), &edit, kind)
        {
            last.edits.push(edit);
            last.bytes += add;
            self.bytes += add;
            self.trim();
            return;
        }
        self.undo.push_back(Group {
            edits: vec![edit],
            bytes: add,
            kind,
        });
        self.bytes += add;
        self.trim();
    }

    /// Records edits that undo together (replace-all, a line move).
    pub fn record_group(&mut self, edits: Vec<EditRecord>) {
        if edits.is_empty() {
            return;
        }
        self.redo.clear();
        let bytes: usize = edits.iter().map(edit_bytes).sum();
        self.undo.push_back(Group {
            edits,
            bytes,
            kind: Coalesce::No,
        });
        self.bytes += bytes;
        self.trim();
    }

    /// Ends a typing/deleting run so the next edit starts a new step.
    pub fn break_run(&mut self) {
        if let Some(last) = self.undo.back_mut() {
            last.kind = Coalesce::No;
        }
    }

    /// Undoes the newest step and returns what changed, or `None` when there is nothing.
    /// If the buffer was changed outside the log the history no longer fits: it is
    /// cleared and the error returned.
    pub fn undo(&mut self, buf: &mut Buffer) -> Result<Option<Vec<EditRecord>>, EditError> {
        let Some(group) = self.undo.pop_back() else {
            return Ok(None);
        };
        self.bytes = self.bytes.saturating_sub(group.bytes);
        let mut applied = Vec::with_capacity(group.edits.len());
        for e in group.edits.iter().rev() {
            let end = e.start_char + e.inserted.chars().count();
            match buf.replace(e.start_char..end, &e.removed) {
                Ok(r) => applied.push(r),
                Err(err) => {
                    self.clear();
                    return Err(err);
                }
            }
        }
        self.redo.push(Group {
            kind: Coalesce::No,
            ..group
        });
        Ok(Some(applied))
    }

    pub fn redo(&mut self, buf: &mut Buffer) -> Result<Option<Vec<EditRecord>>, EditError> {
        let Some(group) = self.redo.pop() else {
            return Ok(None);
        };
        let mut applied = Vec::with_capacity(group.edits.len());
        for e in &group.edits {
            let end = e.start_char + e.removed.chars().count();
            match buf.replace(e.start_char..end, &e.inserted) {
                Ok(r) => applied.push(r),
                Err(err) => {
                    self.clear();
                    return Err(err);
                }
            }
        }
        self.bytes += group.bytes;
        self.undo.push_back(group);
        self.trim();
        Ok(Some(applied))
    }

    fn trim(&mut self) {
        while self.undo.len() > self.max_steps
            || (self.bytes > self.max_bytes && self.undo.len() > 1)
        {
            let Some(old) = self.undo.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(old.bytes);
            self.truncated = true;
        }
    }
}

fn continues(prev: Option<&EditRecord>, next: &EditRecord, kind: Coalesce) -> bool {
    let Some(prev) = prev else {
        return false;
    };
    match kind {
        Coalesce::No => false,
        Coalesce::Typing => {
            next.removed.is_empty()
                && !next.inserted.contains('\n')
                && prev.removed.is_empty()
                && !prev.inserted.contains('\n')
                && next.start_char == prev.start_char + prev.inserted.chars().count()
        }
        Coalesce::Deleting => {
            let removed = next.removed.chars().count();
            next.inserted.is_empty()
                && prev.inserted.is_empty()
                // Backspace continues leftwards, Delete stays at the same index.
                && (next.start_char + removed == prev.start_char
                    || next.start_char == prev.start_char)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn type_str(buf: &mut Buffer, log: &mut UndoLog, s: &str) {
        for c in s.chars() {
            let at = buf.len_chars();
            let r = buf.insert(at, &c.to_string()).expect("insert");
            log.record(r, Coalesce::Typing);
        }
    }

    #[test]
    fn typing_run_is_one_step() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::default();
        type_str(&mut buf, &mut log, "hello");
        assert_eq!(log.undo_steps(), 1);
        log.undo(&mut buf).expect("undo");
        assert_eq!(buf.to_text(), "");
        log.redo(&mut buf).expect("redo");
        assert_eq!(buf.to_text(), "hello");
    }

    #[test]
    fn newline_and_break_start_new_steps() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::default();
        type_str(&mut buf, &mut log, "ab");
        let r = buf.insert(2, "\n").expect("nl");
        log.record(r, Coalesce::Typing);
        type_str(&mut buf, &mut log, "cd");
        assert_eq!(log.undo_steps(), 3);
        log.break_run();
        type_str(&mut buf, &mut log, "e");
        assert_eq!(log.undo_steps(), 4);
    }

    #[test]
    fn backspace_run_is_one_step() {
        let mut buf = Buffer::from_text("abcd");
        let mut log = UndoLog::default();
        for end in (1..=4).rev() {
            let r = buf.remove(end - 1..end).expect("remove");
            log.record(r, Coalesce::Deleting);
        }
        assert_eq!(log.undo_steps(), 1);
        log.undo(&mut buf).expect("undo");
        assert_eq!(buf.to_text(), "abcd");
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::default();
        let r = buf.insert(0, "a").expect("a");
        log.record(r, Coalesce::No);
        log.undo(&mut buf).expect("undo");
        assert!(log.can_redo());
        let r = buf.insert(0, "b").expect("b");
        log.record(r, Coalesce::No);
        assert!(!log.can_redo());
    }

    #[test]
    fn step_limit_drops_oldest_and_is_configurable() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::new(50);
        for i in 0..80 {
            let r = buf.insert(buf.len_chars(), &format!("{i},")).expect("ins");
            log.record(r, Coalesce::No);
        }
        assert_eq!(log.undo_steps(), 50);
        assert!(log.truncated());
        log.set_max_steps(20);
        assert_eq!(log.max_steps(), 20);
        assert_eq!(log.undo_steps(), 20);
        let mut undone = 0;
        while log.undo(&mut buf).expect("undo").is_some() {
            undone += 1;
        }
        assert_eq!(undone, 20);
        assert!(buf.to_text().starts_with("0,1,"));
    }

    #[test]
    fn step_limit_is_clamped() {
        assert_eq!(UndoLog::new(0).max_steps(), MIN_STEPS);
        assert_eq!(UndoLog::new(usize::MAX).max_steps(), MAX_STEPS);
    }

    #[test]
    fn byte_cap_drops_oldest_but_keeps_the_newest() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::new(1000);
        log.set_max_bytes(100);
        for _ in 0..10 {
            let r = buf.insert(0, &"x".repeat(40)).expect("ins");
            log.record(r, Coalesce::No);
        }
        assert!(log.undo_steps() <= 3 && log.undo_steps() >= 1);
        let big = buf.insert(0, &"y".repeat(500)).expect("big");
        log.record(big, Coalesce::No);
        assert_eq!(log.undo_steps(), 1);
        log.undo(&mut buf).expect("undo");
        assert_eq!(buf.len_chars(), 400);
    }

    #[test]
    fn group_undoes_together() {
        let mut buf = Buffer::from_text("a a a");
        let mut log = UndoLog::default();
        let mut recs = Vec::new();
        for start in [4usize, 2, 0] {
            recs.push(buf.replace(start..start + 1, "bb").expect("rep"));
        }
        log.record_group(recs);
        assert_eq!(buf.to_text(), "bb bb bb");
        log.undo(&mut buf).expect("undo");
        assert_eq!(buf.to_text(), "a a a");
        log.redo(&mut buf).expect("redo");
        assert_eq!(buf.to_text(), "bb bb bb");
    }

    #[test]
    fn outside_change_clears_history_with_an_error() {
        let mut buf = Buffer::new();
        let mut log = UndoLog::default();
        let r = buf.insert(0, "hello").expect("ins");
        log.record(r, Coalesce::No);
        buf.remove(0..5).expect("outside change");
        assert!(log.undo(&mut buf).is_err());
        assert!(!log.can_undo() && !log.can_redo());
    }

    proptest! {
        #[test]
        fn undo_all_then_redo_all_round_trips(
            ops in proptest::collection::vec((0usize..40, 0usize..6, "[a-z\\n]{0,6}"), 1..60)
        ) {
            let mut buf = Buffer::from_text("seed text\nline two\n");
            let original = buf.to_text();
            let mut log = UndoLog::new(1000);
            for (at, len, text) in ops {
                let total = buf.len_chars();
                let start = at.min(total);
                let end = (start + len).min(total);
                let rec = buf.replace(start..end, &text).expect("in range");
                log.record(rec, Coalesce::No);
            }
            let edited = buf.to_text();
            while log.undo(&mut buf).expect("undo").is_some() {}
            prop_assert_eq!(buf.to_text(), original);
            while log.redo(&mut buf).expect("redo").is_some() {}
            prop_assert_eq!(buf.to_text(), edited);
        }
    }
}
