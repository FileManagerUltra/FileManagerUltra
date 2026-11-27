//! Rope-backed text buffer. Every position is bounds-checked and an invalid one is an
//! error, never a panic (golden rules 1 and 8). Lines end at `\n` or `\r\n` only.

use std::fmt;
use std::ops::Range;

use ropey::{Rope, RopeSlice};

/// A position as 0-based line and 0-based column counted in `char`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditError {
    /// A line, column or char index outside the buffer.
    OutOfRange,
    /// A range whose start is after its end.
    BadRange,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange => f.write_str("position is outside the text"),
            Self::BadRange => f.write_str("range start is after its end"),
        }
    }
}

impl std::error::Error for EditError {}

/// What one edit changed, in the units the syntax tree and the undo log need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRecord {
    pub start_char: usize,
    pub start_byte: usize,
    pub old_end_byte: usize,
    pub new_end_byte: usize,
    pub start: Pos,
    pub old_end: Pos,
    pub new_end: Pos,
    /// The text that was removed.
    pub removed: String,
    /// The text that was inserted.
    pub inserted: String,
}

/// The text of one file. `version` grows on every change so followers (syntax, live
/// preview) can tell whether they are current.
#[derive(Clone, Debug, Default)]
pub struct Buffer {
    rope: Rope,
    version: u64,
}

impl Buffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_text(text: &str) -> Self {
        Self {
            rope: Rope::from_str(text),
            version: 0,
        }
    }

    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// Number of lines; a trailing newline starts an empty last line, and an empty
    /// buffer has one empty line.
    pub fn len_lines(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn is_empty(&self) -> bool {
        self.rope.len_chars() == 0
    }

    /// Appends text read in chunks (streaming load). Counts as one change.
    pub fn append(&mut self, text: &str) {
        let end = self.rope.len_chars();
        // The end index is always valid, so this cannot fail.
        let _ = self.rope.try_insert(end, text);
        self.version += 1;
    }

    /// The line without its line ending.
    pub fn line_text(&self, line: usize) -> Option<String> {
        let slice = self.rope.get_line(line)?;
        Some(trim_eol(slice).to_string())
    }

    /// Length of the line in chars, without its ending.
    pub fn line_len(&self, line: usize) -> Option<usize> {
        self.rope.get_line(line).map(|s| trim_eol(s).len_chars())
    }

    /// The line ending of a line: `"\r\n"`, `"\n"`, or `""` for the last line.
    pub fn line_ending(&self, line: usize) -> Option<&'static str> {
        let slice = self.rope.get_line(line)?;
        let len = slice.len_chars();
        if len > 0 && slice.get_char(len - 1) == Some('\n') {
            if len > 1 && slice.get_char(len - 2) == Some('\r') {
                Some("\r\n")
            } else {
                Some("\n")
            }
        } else {
            Some("")
        }
    }

    /// Char index where a line starts (`len_chars()` for the line after the last).
    pub fn line_start(&self, line: usize) -> Result<usize, EditError> {
        self.rope
            .try_line_to_char(line)
            .map_err(|_| EditError::OutOfRange)
    }

    pub fn char_at(&self, idx: usize) -> Option<char> {
        self.rope.get_char(idx)
    }

    /// Chars from `idx` to the end.
    pub fn chars_from(&self, idx: usize) -> Option<impl Iterator<Item = char> + '_> {
        (idx <= self.rope.len_chars()).then(|| self.rope.chars_at(idx))
    }

    /// Chars before `idx`, nearest first.
    pub fn chars_before(&self, idx: usize) -> Option<impl Iterator<Item = char> + '_> {
        (idx <= self.rope.len_chars()).then(|| {
            let mut chars = self.rope.chars_at(idx);
            chars.reverse();
            chars
        })
    }

    /// Char index of a position; a column past the end of the line is clamped to it.
    pub fn pos_to_char(&self, pos: Pos) -> Result<usize, EditError> {
        let len = self.line_len(pos.line).ok_or(EditError::OutOfRange)?;
        let start = self
            .rope
            .try_line_to_char(pos.line)
            .map_err(|_| EditError::OutOfRange)?;
        Ok(start + pos.col.min(len))
    }

    pub fn char_to_pos(&self, idx: usize) -> Result<Pos, EditError> {
        let line = self
            .rope
            .try_char_to_line(idx)
            .map_err(|_| EditError::OutOfRange)?;
        let start = self
            .rope
            .try_line_to_char(line)
            .map_err(|_| EditError::OutOfRange)?;
        Ok(Pos {
            line,
            col: idx - start,
        })
    }

    pub fn char_to_byte(&self, idx: usize) -> Result<usize, EditError> {
        self.rope
            .try_char_to_byte(idx)
            .map_err(|_| EditError::OutOfRange)
    }

    /// The text of a char range.
    pub fn slice(&self, range: Range<usize>) -> Result<String, EditError> {
        if range.start > range.end {
            return Err(EditError::BadRange);
        }
        self.rope
            .get_slice(range)
            .map(|s| s.to_string())
            .ok_or(EditError::OutOfRange)
    }

    pub fn to_text(&self) -> String {
        self.rope.to_string()
    }

    /// Streams the buffer out chunk by chunk (for saving without one big allocation).
    pub fn chunks(&self) -> impl Iterator<Item = &str> {
        self.rope.chunks()
    }

    /// The rest of the chunk that holds `byte` (empty at or past the end); lets a parser
    /// read the rope without copying it.
    pub fn text_from_byte(&self, byte: usize) -> &str {
        if byte >= self.rope.len_bytes() {
            return "";
        }
        let (chunk, chunk_start, _, _) = self.rope.chunk_at_byte(byte);
        chunk.get(byte - chunk_start..).unwrap_or("")
    }

    /// Chunks covering a byte range, clamped to the buffer.
    pub fn byte_chunks(&self, range: Range<usize>) -> impl Iterator<Item = &str> {
        let len = self.rope.len_bytes();
        let end = range.end.min(len);
        let start = range.start.min(end);
        self.rope
            .get_byte_slice(start..end)
            .into_iter()
            .flat_map(|s| s.chunks())
    }

    /// Replaces a char range with `text` and reports exactly what changed.
    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<EditRecord, EditError> {
        if range.start > range.end {
            return Err(EditError::BadRange);
        }
        if range.end > self.rope.len_chars() {
            return Err(EditError::OutOfRange);
        }
        let start_char = range.start;
        let start_byte = self.char_to_byte(start_char)?;
        let old_end_byte = self.char_to_byte(range.end)?;
        let start = self.char_to_pos(start_char)?;
        let old_end = self.char_to_pos(range.end)?;
        let removed = self.slice(range.clone())?;

        self.rope
            .try_remove(range)
            .map_err(|_| EditError::OutOfRange)?;
        self.rope
            .try_insert(start_char, text)
            .map_err(|_| EditError::OutOfRange)?;
        self.version += 1;

        let inserted_chars = text.chars().count();
        let new_end = self.char_to_pos(start_char + inserted_chars)?;
        Ok(EditRecord {
            start_char,
            start_byte,
            old_end_byte,
            new_end_byte: start_byte + text.len(),
            start,
            old_end,
            new_end,
            removed,
            inserted: text.to_string(),
        })
    }

    pub fn insert(&mut self, at: usize, text: &str) -> Result<EditRecord, EditError> {
        self.replace(at..at, text)
    }

    pub fn remove(&mut self, range: Range<usize>) -> Result<EditRecord, EditError> {
        self.replace(range, "")
    }
}

fn trim_eol(line: RopeSlice<'_>) -> RopeSlice<'_> {
    let len = line.len_chars();
    let mut end = len;
    if end > 0 && line.get_char(end - 1) == Some('\n') {
        end -= 1;
    }
    if end > 0 && line.get_char(end - 1) == Some('\r') {
        end -= 1;
    }
    line.slice(..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_positions() {
        let b = Buffer::from_text("ab\r\ncd\n");
        assert_eq!(b.len_lines(), 3);
        assert_eq!(b.line_text(0).as_deref(), Some("ab"));
        assert_eq!(b.line_text(1).as_deref(), Some("cd"));
        assert_eq!(b.line_text(2).as_deref(), Some(""));
        assert_eq!(b.line_text(3), None);
        assert_eq!(b.pos_to_char(Pos { line: 1, col: 1 }), Ok(5));
        assert_eq!(b.pos_to_char(Pos { line: 1, col: 99 }), Ok(6));
        assert_eq!(
            b.pos_to_char(Pos { line: 9, col: 0 }),
            Err(EditError::OutOfRange)
        );
        assert_eq!(b.char_to_pos(5), Ok(Pos { line: 1, col: 1 }));
    }

    #[test]
    fn empty_buffer_has_one_line() {
        let b = Buffer::new();
        assert_eq!(b.len_lines(), 1);
        assert_eq!(b.line_text(0).as_deref(), Some(""));
        assert!(b.is_empty());
    }

    #[test]
    fn edits_report_what_changed() {
        let mut b = Buffer::from_text("héllo\nworld");
        let r = b.replace(1..3, "EY").expect("replace");
        assert_eq!(b.to_text(), "hEYlo\nworld");
        assert_eq!(r.removed, "él");
        assert_eq!(r.start_byte, 1);
        assert_eq!(r.old_end_byte, 4);
        assert_eq!(r.new_end_byte, 3);
        assert_eq!(r.new_end, Pos { line: 0, col: 3 });
        assert_eq!(b.version(), 1);
    }

    #[test]
    fn insert_newline_moves_end_to_next_line() {
        let mut b = Buffer::from_text("ab");
        let r = b.insert(1, "x\ny").expect("insert");
        assert_eq!(b.to_text(), "ax\nyb");
        assert_eq!(r.new_end, Pos { line: 1, col: 1 });
    }

    #[test]
    fn bad_ranges_are_errors_and_leave_text_alone() {
        let mut b = Buffer::from_text("abc");
        assert_eq!(b.replace(2..9, "x"), Err(EditError::OutOfRange));
        #[allow(clippy::reversed_empty_ranges)]
        let reversed = 2..1;
        assert_eq!(b.replace(reversed, "x"), Err(EditError::BadRange));
        assert_eq!(b.insert(4, "x"), Err(EditError::OutOfRange));
        assert_eq!(b.to_text(), "abc");
        assert_eq!(b.version(), 0);
    }

    #[test]
    fn append_streams_chunks() {
        let mut b = Buffer::new();
        b.append("one\ntw");
        b.append("o\nthree");
        assert_eq!(b.len_lines(), 3);
        assert_eq!(b.line_text(1).as_deref(), Some("two"));
        assert_eq!(b.chunks().collect::<String>(), "one\ntwo\nthree");
    }

    #[test]
    fn million_lines_stay_addressable() {
        let mut b = Buffer::new();
        for _ in 0..1000 {
            b.append(&"line\n".repeat(1000));
        }
        assert_eq!(b.len_lines(), 1_000_001);
        let at = b
            .pos_to_char(Pos {
                line: 500_000,
                col: 2,
            })
            .expect("pos");
        b.insert(at, "X").expect("insert");
        assert_eq!(b.line_text(500_000).as_deref(), Some("liXne"));
    }
}
