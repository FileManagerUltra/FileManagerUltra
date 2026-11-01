//! Line diff model with intra-line (word) highlights, built for a side-by-side view.
//!
//! Inputs are untrusted: sizes are capped before any work, a deadline bounds the diff
//! algorithm, and binary data is detected instead of decoded.

use std::time::Duration;

use similar::{ChangeTag, DiffOp, TextDiff};

use crate::MAX_BLOB;

/// Most lines per side; beyond this the comparison is refused.
pub const MAX_LINES: usize = 100_000;
/// Word-level highlighting is skipped for longer lines.
const WORD_DIFF_MAX: usize = 1_000;
const DEADLINE: Duration = Duration::from_secs(2);

/// How a row differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Equal,
    /// Only on the left.
    Delete,
    /// Only on the right.
    Insert,
    /// On both sides with different text.
    Replace,
}

/// One displayed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// 1-based line number in its file.
    pub no: u32,
    pub text: String,
    /// Changed byte ranges inside `text` (only for [`Tag::Replace`]).
    pub marks: Vec<(u32, u32)>,
}

/// An aligned row: either side may be absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub left: Option<Line>,
    pub right: Option<Line>,
    pub tag: Tag,
}

/// The aligned result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffModel {
    pub rows: Vec<Row>,
    /// Index of the first row of every change block, ascending.
    pub changes: Vec<usize>,
    pub added: usize,
    pub removed: usize,
}

impl DiffModel {
    pub fn identical(&self) -> bool {
        self.changes.is_empty()
    }
}

/// One side's content as loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    /// The file does not exist on this side (added or deleted).
    Missing,
    Bytes(Vec<u8>),
}

/// Result of comparing two contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Text(DiffModel),
    /// At least one side is binary: only equality is reported.
    Binary {
        same: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffError(pub String);

impl std::fmt::Display for DiffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DiffError {}

fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

fn decode(c: &Content, ignore_eol: bool) -> Result<String, DiffError> {
    let bytes: &[u8] = match c {
        Content::Missing => &[],
        Content::Bytes(b) => b,
    };
    if bytes.len() > MAX_BLOB {
        return Err(DiffError(format!(
            "file larger than {} MiB",
            MAX_BLOB / 1024 / 1024
        )));
    }
    let text = String::from_utf8_lossy(bytes);
    let text = if ignore_eol {
        text.replace("\r\n", "\n")
    } else {
        text.into_owned()
    };
    if text.bytes().filter(|b| *b == b'\n').count() > MAX_LINES {
        return Err(DiffError(format!("more than {MAX_LINES} lines")));
    }
    Ok(text)
}

/// Compare two contents line by line.
pub fn diff(left: &Content, right: &Content, ignore_eol: bool) -> Result<Outcome, DiffError> {
    for c in [left, right] {
        if let Content::Bytes(b) = c
            && is_binary(b)
        {
            return Ok(Outcome::Binary {
                same: left == right,
            });
        }
    }
    let a = decode(left, ignore_eol)?;
    let b = decode(right, ignore_eol)?;
    let a_lines: Vec<&str> = a.split_inclusive('\n').collect();
    let b_lines: Vec<&str> = b.split_inclusive('\n').collect();
    let text_diff = TextDiff::configure().timeout(DEADLINE).diff_lines(&a, &b);
    let mut model = DiffModel::default();
    let strip = |s: &str| s.trim_end_matches(['\n', '\r']).to_string();
    let line = |lines: &[&str], i: usize, marks: Vec<(u32, u32)>| Line {
        no: (i + 1) as u32,
        text: lines.get(i).map(|s| strip(s)).unwrap_or_default(),
        marks,
    };
    for op in text_diff.ops() {
        match *op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for k in 0..len {
                    model.rows.push(Row {
                        left: Some(line(&a_lines, old_index + k, Vec::new())),
                        right: Some(line(&b_lines, new_index + k, Vec::new())),
                        tag: Tag::Equal,
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                model.changes.push(model.rows.len());
                for k in 0..old_len {
                    model.removed += 1;
                    model.rows.push(Row {
                        left: Some(line(&a_lines, old_index + k, Vec::new())),
                        right: None,
                        tag: Tag::Delete,
                    });
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                model.changes.push(model.rows.len());
                for k in 0..new_len {
                    model.added += 1;
                    model.rows.push(Row {
                        left: None,
                        right: Some(line(&b_lines, new_index + k, Vec::new())),
                        tag: Tag::Insert,
                    });
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                model.changes.push(model.rows.len());
                for k in 0..old_len.max(new_len) {
                    let l = (k < old_len).then(|| old_index + k);
                    let r = (k < new_len).then(|| new_index + k);
                    let (lm, rm) = match (l, r) {
                        (Some(l), Some(r)) => word_marks(
                            &strip(a_lines.get(l).copied().unwrap_or_default()),
                            &strip(b_lines.get(r).copied().unwrap_or_default()),
                        ),
                        _ => (Vec::new(), Vec::new()),
                    };
                    let tag = match (l, r) {
                        (Some(_), Some(_)) => Tag::Replace,
                        (Some(_), None) => Tag::Delete,
                        _ => Tag::Insert,
                    };
                    model.removed += usize::from(l.is_some());
                    model.added += usize::from(r.is_some());
                    model.rows.push(Row {
                        left: l.map(|i| line(&a_lines, i, lm)),
                        right: r.map(|i| line(&b_lines, i, rm)),
                        tag,
                    });
                }
            }
        }
    }
    Ok(Outcome::Text(model))
}

/// Changed byte ranges inside a pair of lines.
/// Changed byte ranges of one line.
type Marks = Vec<(u32, u32)>;

fn word_marks(a: &str, b: &str) -> (Marks, Marks) {
    if a.len() > WORD_DIFF_MAX || b.len() > WORD_DIFF_MAX {
        return (Vec::new(), Vec::new());
    }
    let d = TextDiff::configure()
        .timeout(Duration::from_millis(50))
        .diff_words(a, b);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    let (mut lo, mut ro) = (0usize, 0usize);
    let push = |v: &mut Vec<(u32, u32)>, start: usize, len: usize| match v.last_mut() {
        Some(last) if last.1 as usize == start => last.1 = (start + len) as u32,
        _ => v.push((start as u32, (start + len) as u32)),
    };
    for change in d.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                lo += len;
                ro += len;
            }
            ChangeTag::Delete => {
                push(&mut left, lo, len);
                lo += len;
            }
            ChangeTag::Insert => {
                push(&mut right, ro, len);
                ro += len;
            }
        }
    }
    (left, right)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Content {
        Content::Bytes(s.as_bytes().to_vec())
    }

    fn model(a: &str, b: &str) -> DiffModel {
        match diff(&text(a), &text(b), false).unwrap_or(Outcome::Binary { same: true }) {
            Outcome::Text(m) => m,
            Outcome::Binary { .. } => DiffModel::default(),
        }
    }

    #[test]
    fn identical_has_no_changes() {
        let m = model("a\nb\n", "a\nb\n");
        assert!(m.identical());
        assert_eq!(m.rows.len(), 2);
    }

    #[test]
    fn insert_delete_and_replace_align() {
        let m = model("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(m.changes, vec![1, 3]);
        assert_eq!(m.rows[1].tag, Tag::Replace);
        assert_eq!(m.rows[3].tag, Tag::Insert);
        assert!(m.rows[3].left.is_none());
        assert_eq!((m.added, m.removed), (2, 1));
    }

    #[test]
    fn words_are_marked_inside_a_replaced_line() {
        let m = model("let x = 1;\n", "let x = 22;\n");
        let right = m.rows[0].right.as_ref().map(|l| l.marks.clone());
        let marks = right.unwrap_or_default();
        assert!(!marks.is_empty());
        let (s, e) = marks[0];
        assert!(
            m.rows[0]
                .right
                .as_ref()
                .is_some_and(|l| l.text[s as usize..e as usize].contains("22"))
        );
    }

    #[test]
    fn missing_side_is_all_inserts() {
        let out = diff(&Content::Missing, &text("x\ny\n"), false);
        match out {
            Ok(Outcome::Text(m)) => assert_eq!((m.added, m.removed), (2, 0)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn ignoring_line_endings_makes_crlf_equal() {
        assert!(
            matches!(diff(&text("a\r\nb\r\n"), &text("a\nb\n"), true), Ok(Outcome::Text(m)) if m.identical())
        );
        assert!(
            matches!(diff(&text("a\r\n"), &text("a\n"), false), Ok(Outcome::Text(m)) if !m.identical())
        );
    }

    #[test]
    fn binary_is_detected_not_decoded() {
        let bin = Content::Bytes(vec![0, 1, 2]);
        assert_eq!(
            diff(&bin, &bin.clone(), false),
            Ok(Outcome::Binary { same: true })
        );
        assert_eq!(
            diff(&bin, &text("x"), false),
            Ok(Outcome::Binary { same: false })
        );
    }

    #[test]
    fn caps_reject_huge_inputs() {
        let big = Content::Bytes(vec![b'a'; MAX_BLOB + 1]);
        assert!(diff(&big, &Content::Missing, false).is_err());
        let many = Content::Bytes("\n".repeat(MAX_LINES + 1).into_bytes());
        assert!(diff(&many, &Content::Missing, false).is_err());
    }

    #[test]
    fn invalid_utf8_is_decoded_lossily() {
        let bad = Content::Bytes(vec![b'a', 0xff, b'\n']);
        assert!(diff(&bad, &text("a\n"), false).is_ok());
    }
}
