//! Rainbow brackets (change `add-editor-folding-live-preview`): the nesting depth of
//! every bracket in a line range, from the syntax tree. Brackets inside strings and
//! comments are not tokens of their own in the tree, so they are never counted.
//! Without a tree there is no rainbow (no guessing).

use std::ops::Range;

use tree_sitter::{Node, Tree};

use crate::buffer::Buffer;

/// One bracket: byte offsets relative to its line, nesting depth from 0, and whether
/// it has no partner (an error the parser recovered from).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BracketSpan {
    pub start: usize,
    pub end: usize,
    pub depth: usize,
    pub unmatched: bool,
}

/// Nodes visited at most per call; deeper or larger trees just show fewer colours.
const MAX_VISIT: usize = 200_000;

fn is_open(kind: &str) -> bool {
    matches!(kind, "(" | "[" | "{")
}

fn is_bracket(kind: &str) -> bool {
    matches!(kind, "(" | ")" | "[" | "]" | "{" | "}")
}

/// Brackets in `lines` (first included, last excluded), one `Vec` per line.
pub fn rainbow(tree: &Tree, buf: &Buffer, lines: Range<usize>) -> Vec<Vec<BracketSpan>> {
    let last = lines.end.min(buf.len_lines());
    let first = lines.start.min(last);
    let mut out = vec![Vec::new(); last - first];
    if first == last {
        return out;
    }
    let (Ok(lo), Ok(hi)) = (
        buf.line_start(first).and_then(|c| buf.char_to_byte(c)),
        buf.line_start(last)
            .or_else(|_| Ok::<usize, crate::buffer::EditError>(buf.len_chars()))
            .and_then(|c| buf.char_to_byte(c)),
    ) else {
        return out;
    };
    let line_byte = |line: usize| -> usize {
        buf.line_start(line)
            .and_then(|c| buf.char_to_byte(c))
            .unwrap_or(0)
    };
    let starts: Vec<usize> = (first..last).map(line_byte).collect();
    // (node, depth of the brackets that are its own children, inside an error)
    let mut stack: Vec<(Node<'_>, usize, bool)> = vec![(tree.root_node(), 0, false)];
    let mut visited = 0usize;
    while let Some((node, depth, in_error)) = stack.pop() {
        visited += 1;
        if visited > MAX_VISIT {
            break;
        }
        if node.end_byte() < lo || node.start_byte() > hi {
            continue;
        }
        let in_error = in_error || node.is_error();
        let mut cursor = node.walk();
        let children: Vec<Node<'_>> = node.children(&mut cursor).collect();
        let opens = children
            .iter()
            .any(|c| c.child_count() == 0 && !c.is_named() && is_open(c.kind()));
        let inner = if opens { depth + 1 } else { depth };
        // A partner the parser had to invent: this node's real brackets are unmatched.
        let invented = children
            .iter()
            .any(|c| c.is_missing() && is_bracket(c.kind()));
        for child in children.into_iter().rev() {
            let kind = child.kind();
            if child.child_count() == 0 && is_bracket(kind) {
                let (s, e) = (child.start_byte(), child.end_byte());
                if s < lo || s >= hi {
                    continue;
                }
                let row = child.start_position().row;
                let Some(ix) = row.checked_sub(first) else {
                    continue;
                };
                let (Some(line_start), Some(spans)) = (starts.get(ix), out.get_mut(ix)) else {
                    continue;
                };
                spans.push(BracketSpan {
                    start: s - line_start,
                    end: e - line_start,
                    depth,
                    unmatched: in_error || invented || child.is_missing(),
                });
            } else if child.child_count() > 0 {
                stack.push((child, inner, in_error));
            }
        }
    }
    for spans in &mut out {
        spans.sort_unstable_by_key(|s| s.start);
        // A zero-width MISSING bracket is not drawn.
        spans.retain(|s| s.end > s.start);
    }
    out
}

#[cfg(all(test, feature = "lang-rust", feature = "lang-json"))]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use crate::{Lang, ParseCtl, Syntax};

    fn tree_of(lang: Lang, text: &str) -> (Buffer, Tree) {
        let buf = Buffer::from_text(text);
        let syn = Syntax::new(lang).expect("lang");
        let (tree, _) = syn
            .job(&buf, None)
            .run(&ParseCtl::default())
            .expect("parse");
        (buf, tree)
    }

    fn depths(spans: &[BracketSpan]) -> Vec<usize> {
        spans.iter().map(|s| s.depth).collect()
    }

    #[test]
    fn nesting_depths_follow_the_tree() {
        let (buf, tree) = tree_of(Lang::Rust, "fn f(a: [u8; 2]) { g((1)); }\n");
        let lines = rainbow(&tree, &buf, 0..1);
        let line = lines.first().expect("line");
        // ( [ ] ) { ( ( ) ) }
        assert_eq!(depths(line), vec![0, 1, 1, 0, 0, 1, 2, 2, 1, 0]);
        assert!(line.iter().all(|s| !s.unmatched));
    }

    #[test]
    fn strings_and_comments_do_not_count() {
        let (buf, tree) = tree_of(Lang::Rust, "fn f() { let s = \"((\"; // ((\n}\n");
        let lines = rainbow(&tree, &buf, 0..2);
        let count: usize = lines.iter().map(Vec::len).sum();
        assert_eq!(count, 4, "only ( ) {{ }}");
    }

    #[test]
    fn depth_at_a_later_range_is_correct() {
        let (buf, tree) = tree_of(Lang::Json, "{\n  \"a\": [\n    [1, 2]\n  ]\n}\n");
        let lines = rainbow(&tree, &buf, 2..3);
        assert_eq!(depths(lines.first().expect("line")), vec![2, 2]);
    }

    #[test]
    fn unmatched_brackets_are_marked() {
        let (buf, tree) = tree_of(Lang::Rust, "fn f() { g(1; }\n");
        let lines = rainbow(&tree, &buf, 0..1);
        assert!(lines.iter().flatten().any(|s| s.unmatched));
    }
}
