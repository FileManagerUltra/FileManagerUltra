//! Code folding (capability `text-editor`, change `add-editor-folding-live-preview`):
//! which lines can fold, and which are folded.
//!
//! Foldable ranges come from the syntax tree (multi-line blocks, bodies, objects, lists,
//! comments, elements, sections) when there is one, else from indentation. A range is
//! `start..=end` in buffer lines; folding hides `start + 1..=end` and keeps `start`
//! visible with a placeholder. The folded set maps through edits so folds do not jump
//! while typing; an edit inside a folded range unfolds it.

use std::ops::Range;

use tree_sitter::{Node, Tree};

use crate::buffer::{Buffer, EditRecord};

/// Folds must hide at least this many lines.
pub const MIN_FOLD_LINES: usize = 1;
/// Lines an indentation fold looks ahead at most (huge files stay cheap).
const INDENT_SCAN: usize = 200_000;
/// Nodes the tree walk visits at most per computation.
const MAX_NODES: usize = 2_000_000;

/// Node kinds that fold, by part of their name; matched against every grammar's node
/// names, so each language folds its blocks without a query of its own.
const FOLD_KINDS: &[&str] = &[
    "block",
    "body",
    "class",
    "function",
    "method",
    "impl",
    "trait",
    "struct",
    "enum",
    "object",
    "array",
    "list",
    "table",
    "dictionary",
    "comment",
    "element",
    "section",
    "match",
    "switch",
    "statement",
    "mapping",
    "sequence",
    "arguments",
    "parameters",
    "declaration_list",
    "field_declaration_list",
    "use_list",
    "string",
    "heredoc",
];

/// Foldable ranges of a parsed buffer, sorted by start line, one per start line (the
/// longest).
pub fn tree_ranges(tree: &Tree) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    let mut stack: Vec<Node<'_>> = vec![tree.root_node()];
    let mut seen = 0usize;
    while let Some(node) = stack.pop() {
        seen += 1;
        if seen > MAX_NODES {
            break;
        }
        let (start, end) = (node.start_position().row, node.end_position().row);
        if end <= start {
            continue;
        }
        let kind = node.kind();
        if node.is_named() && FOLD_KINDS.iter().any(|k| kind.contains(k)) {
            // A node ending at column 0 ends on the line before (trailing newline).
            let end = if node.end_position().column == 0 {
                end - 1
            } else {
                end
            };
            if end >= start + MIN_FOLD_LINES {
                out.push((start, end));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.end_position().row > child.start_position().row {
                stack.push(child);
            }
        }
    }
    out.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    out.dedup_by_key(|r| r.0);
    out
}

fn indent_of(line: &str) -> Option<usize> {
    let mut width = 0usize;
    for c in line.chars() {
        match c {
            ' ' => width += 1,
            '\t' => width += 4,
            '\r' | '\n' => return None,
            _ => return Some(width),
        }
    }
    None
}

/// The indentation fold starting at `line`: the following lines indented deeper (blank
/// lines included when deeper lines follow them).
pub fn indent_range(buf: &Buffer, line: usize) -> Option<usize> {
    let base = indent_of(&buf.line_text(line)?)?;
    let total = buf.len_lines();
    let mut end = None;
    let limit = (line + INDENT_SCAN).min(total);
    for next in line + 1..limit {
        let text = buf.line_text(next)?;
        match indent_of(&text) {
            None => continue,
            Some(i) if i > base => end = Some(next),
            Some(_) => break,
        }
    }
    end.filter(|e| *e >= line + MIN_FOLD_LINES)
}

/// Where folds can start: tree ranges when the buffer is parsed, else indentation.
#[derive(Clone, Debug, Default)]
pub struct FoldRanges {
    /// From the last parse; `None` means use indentation.
    tree: Option<Vec<(usize, usize)>>,
    /// Buffer version the tree ranges belong to (edits since shift them).
    version: u64,
}

impl FoldRanges {
    pub fn from_tree(ranges: Vec<(usize, usize)>, version: u64) -> Self {
        Self {
            tree: Some(ranges),
            version,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// The fold starting at `line`, if one does.
    pub fn range_at(&self, buf: &Buffer, line: usize) -> Option<usize> {
        match &self.tree {
            Some(ranges) => {
                let ix = ranges.binary_search_by_key(&line, |r| r.0).ok()?;
                ranges
                    .get(ix)
                    .map(|r| r.1.min(buf.len_lines().saturating_sub(1)))
                    .filter(|e| *e > line)
            }
            None => indent_range(buf, line),
        }
    }

    /// Lines in `lines` where a fold starts (for the gutter).
    pub fn starts_in(&self, buf: &Buffer, lines: Range<usize>) -> Vec<usize> {
        match &self.tree {
            Some(ranges) => {
                let from = ranges.partition_point(|r| r.0 < lines.start);
                ranges
                    .iter()
                    .skip(from)
                    .take_while(|r| r.0 < lines.end)
                    .map(|r| r.0)
                    .collect()
            }
            None => lines
                .filter(|l| {
                    let Some(base) = buf.line_text(*l).as_deref().and_then(indent_of) else {
                        return false;
                    };
                    // Cheap test: the next non-blank line is deeper.
                    (l + 1..(l + 64).min(buf.len_lines()))
                        .find_map(|n| buf.line_text(n).as_deref().and_then(indent_of))
                        .is_some_and(|i| i > base)
                })
                .collect(),
        }
    }

    /// Shifts tree ranges through an edit already applied to `buf`, until the next parse
    /// replaces them.
    pub fn note_edit(&mut self, buf: &Buffer, rec: &EditRecord) {
        let Some(ranges) = self.tree.as_mut() else {
            return;
        };
        let Some((line, removed, inserted)) = edit_lines(buf, rec) else {
            return;
        };
        if removed == inserted {
            return;
        }
        let delta = inserted as i64 - removed as i64;
        let shift = |l: usize| usize::try_from(l as i64 + delta).unwrap_or(0);
        ranges.retain_mut(|r| {
            if r.0 > line + removed {
                *r = (shift(r.0), shift(r.1));
                true
            } else if r.1 >= line {
                r.1 = shift(r.1.max(line + removed));
                r.1 > r.0
            } else {
                true
            }
        });
    }
}

/// The line an edit starts on and how many line breaks it removed and inserted.
fn edit_lines(buf: &Buffer, rec: &EditRecord) -> Option<(usize, usize, usize)> {
    let line = buf.char_to_pos(rec.start_char).ok()?.line;
    let removed = rec.removed.matches('\n').count();
    let inserted = rec.inserted.matches('\n').count();
    Some((line, removed, inserted))
}

/// The folded ranges of one editor: sorted, disjoint, each `(start, end)` hiding
/// `start + 1..=end`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldIndex {
    folded: Vec<(usize, usize)>,
}

impl FoldIndex {
    pub fn is_empty(&self) -> bool {
        self.folded.is_empty()
    }

    pub fn ranges(&self) -> &[(usize, usize)] {
        &self.folded
    }

    /// Folded starts, for workspace state.
    pub fn starts(&self) -> Vec<usize> {
        self.folded.iter().map(|r| r.0).collect()
    }

    /// The folded range whose hidden lines contain `line`.
    pub fn hiding(&self, line: usize) -> Option<(usize, usize)> {
        let ix = self.folded.partition_point(|r| r.0 < line);
        let r = *self.folded.get(ix.checked_sub(1)?)?;
        (line > r.0 && line <= r.1).then_some(r)
    }

    pub fn is_hidden(&self, line: usize) -> bool {
        self.hiding(line).is_some()
    }

    /// The folded range starting at `line`.
    pub fn folded_at(&self, line: usize) -> Option<(usize, usize)> {
        let ix = self.folded.binary_search_by_key(&line, |r| r.0).ok()?;
        self.folded.get(ix).copied()
    }

    /// Folds `start..=end`; folds inside it merge into it. Refuses an empty range or one
    /// that starts on a hidden line.
    pub fn fold(&mut self, start: usize, end: usize) -> bool {
        if end <= start || self.is_hidden(start) {
            return false;
        }
        self.folded.retain(|r| !(r.0 >= start && r.0 <= end));
        let at = self.folded.partition_point(|r| r.0 < start);
        self.folded.insert(at, (start, end));
        true
    }

    pub fn unfold(&mut self, start: usize) -> bool {
        let before = self.folded.len();
        self.folded.retain(|r| r.0 != start);
        before != self.folded.len()
    }

    pub fn clear(&mut self) {
        self.folded.clear();
    }

    /// Unfolds whatever hides `line` (the caret jumped there).
    pub fn reveal(&mut self, line: usize) -> bool {
        let mut changed = false;
        while let Some((start, _)) = self.hiding(line) {
            changed |= self.unfold(start);
        }
        changed
    }

    /// The next visible line after `line`.
    pub fn next_visible(&self, line: usize) -> usize {
        match self.folded_at(line) {
            Some((_, end)) => end + 1,
            None => line + 1,
        }
    }

    /// The visible line before `line` (the fold start for a hidden line above).
    pub fn prev_visible(&self, line: usize) -> usize {
        let prev = line.saturating_sub(1);
        self.hiding(prev).map_or(prev, |r| r.0)
    }

    /// A visible line at or above `line`.
    pub fn visible_at(&self, line: usize) -> usize {
        self.hiding(line).map_or(line, |r| r.0)
    }

    /// Maps the folds through an edit already applied to `buf`. Edits in hidden lines,
    /// and line breaks added or removed on a fold's first line, unfold it.
    pub fn note_edit(&mut self, buf: &Buffer, rec: &EditRecord) {
        if self.folded.is_empty() {
            return;
        }
        let Some((line, removed, inserted)) = edit_lines(buf, rec) else {
            self.folded.clear();
            return;
        };
        let delta = inserted as i64 - removed as i64;
        let shift = |l: usize| usize::try_from(l as i64 + delta).unwrap_or(0);
        let last = line + removed;
        self.folded.retain_mut(|r| {
            if r.0 > last {
                *r = (shift(r.0), shift(r.1));
                return true;
            }
            if r.1 < line {
                return true;
            }
            // The edit touches this fold: keep it only for a one-line edit on its first line.
            r.0 == line && removed == 0 && inserted == 0
        });
    }

    /// Restores folds from start lines (a workspace), keeping only ranges that still fold.
    pub fn restore(&mut self, starts: &[usize], ranges: &FoldRanges, buf: &Buffer) {
        self.folded.clear();
        for &start in starts.iter().take(10_000) {
            if let Some(end) = ranges.range_at(buf, start) {
                self.fold(start, end);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn indentation_folds_skip_blank_lines() {
        let buf = Buffer::from_text("a:\n  b\n\n  c\nd\n  e\n");
        assert_eq!(indent_range(&buf, 0), Some(3));
        assert_eq!(indent_range(&buf, 1), None);
        assert_eq!(indent_range(&buf, 4), Some(5));
        let ranges = FoldRanges::default();
        assert_eq!(ranges.starts_in(&buf, 0..6), vec![0, 4]);
    }

    #[test]
    fn folding_maps_lines_and_reveals() {
        let mut f = FoldIndex::default();
        assert!(f.fold(2, 5));
        assert!(f.fold(8, 9));
        assert!(!f.fold(3, 4), "starts on a hidden line");
        assert_eq!(f.next_visible(2), 6);
        assert_eq!(f.prev_visible(6), 2);
        assert_eq!(f.visible_at(4), 2);
        assert!(f.is_hidden(5) && !f.is_hidden(2) && !f.is_hidden(6));
        assert!(f.fold(1, 9), "an outer fold absorbs inner ones");
        assert_eq!(f.ranges(), &[(1, 9)]);
        assert!(f.reveal(4));
        assert!(f.is_empty());
    }

    #[test]
    fn edits_shift_or_unfold() {
        let mut buf = Buffer::from_text("0\n1\n2\n3\n4\n5\n");
        let mut f = FoldIndex::default();
        f.fold(2, 4);
        let rec = buf.insert(0, "x\n").expect("insert");
        f.note_edit(&buf, &rec);
        assert_eq!(f.ranges(), &[(3, 5)], "lines added above shift the fold");
        let at = buf.line_start(3).expect("line");
        let rec = buf.insert(at, "!").expect("insert");
        f.note_edit(&buf, &rec);
        assert_eq!(f.ranges(), &[(3, 5)], "typing on the first line keeps it");
        let at = buf.line_start(4).expect("line");
        let rec = buf.insert(at, "y").expect("insert");
        f.note_edit(&buf, &rec);
        assert!(f.is_empty(), "an edit in hidden lines unfolds");
    }

    #[test]
    fn a_million_lines_look_up_fast() {
        let mut f = FoldIndex::default();
        for i in 0..100_000 {
            f.fold(i * 10, i * 10 + 5);
        }
        let started = std::time::Instant::now();
        let mut line = 0usize;
        let mut shown = 0usize;
        while line < 1_000_000 {
            line = f.next_visible(line);
            shown += 1;
        }
        assert_eq!(shown, 500_000);
        assert!(f.is_hidden(999_993));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn restore_keeps_only_valid_starts() {
        let buf = Buffer::from_text("a\n  b\n  c\nd\n");
        let mut f = FoldIndex::default();
        f.restore(&[0, 1, 99], &FoldRanges::default(), &buf);
        assert_eq!(f.ranges(), &[(0, 2)]);
    }
}
