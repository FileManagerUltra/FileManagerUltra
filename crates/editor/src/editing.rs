//! Editing helpers that turn a keystroke into buffer edits: auto-indent, indent and
//! outdent, line duplicate/delete/move, comment toggling and bracket matching. Each
//! returns the `EditRecord`s it applied (last edit first when there are several), ready
//! for `UndoLog::record_group`.

use crate::buffer::{Buffer, EditError, EditRecord};
use crate::encoding::Eol;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentStyle {
    Spaces(u8),
    Tab,
}

impl IndentStyle {
    pub fn unit(self) -> String {
        match self {
            Self::Spaces(n) => " ".repeat(usize::from(n)),
            Self::Tab => "\t".to_string(),
        }
    }

    fn width(self) -> usize {
        match self {
            Self::Spaces(n) => usize::from(n),
            Self::Tab => 1,
        }
    }
}

/// Guesses the file's indent from its first lines; 4 spaces when there is no signal.
pub fn detect_indent(buf: &Buffer) -> IndentStyle {
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    let mut smallest = usize::MAX;
    for line in 0..buf.len_lines().min(2000) {
        let Some(text) = buf.line_text(line) else {
            break;
        };
        if text.trim().is_empty() {
            continue;
        }
        if text.starts_with('\t') {
            tabs += 1;
        } else {
            let n = text.chars().take_while(|&c| c == ' ').count();
            if n > 0 {
                spaces += 1;
                smallest = smallest.min(n);
            }
        }
    }
    if tabs > spaces {
        IndentStyle::Tab
    } else if (2..=8).contains(&smallest) {
        IndentStyle::Spaces(u8::try_from(smallest).unwrap_or(4))
    } else {
        IndentStyle::Spaces(4)
    }
}

fn leading_ws(s: &str) -> String {
    s.chars().take_while(|&c| c == ' ' || c == '\t').collect()
}

/// The text to insert for Enter at char `at`: the line ending plus the current line's
/// indent, one unit deeper after an opening bracket (or a colon when `colon_opens`).
pub fn newline_text(
    buf: &Buffer,
    at: usize,
    style: IndentStyle,
    eol: Eol,
    colon_opens: bool,
) -> Result<String, EditError> {
    let pos = buf.char_to_pos(at)?;
    let line = buf.line_text(pos.line).ok_or(EditError::OutOfRange)?;
    let before: String = line.chars().take(pos.col).collect();
    let mut out = String::from(eol.as_str());
    out.push_str(&leading_ws(&before));
    let last = before.trim_end().chars().last();
    let opens = match last {
        Some('{' | '(' | '[') => true,
        Some(':') => colon_opens,
        _ => false,
    };
    if opens {
        out.push_str(&style.unit());
    }
    Ok(out)
}

fn line_range(buf: &Buffer, first: usize, last: usize) -> Result<(usize, usize), EditError> {
    let last_line = buf.len_lines() - 1;
    if first > last || first > last_line {
        return Err(EditError::OutOfRange);
    }
    Ok((first, last.min(last_line)))
}

/// Indents (or outdents) lines `first..=last`; empty lines are left alone.
pub fn indent_lines(
    buf: &mut Buffer,
    first: usize,
    last: usize,
    style: IndentStyle,
    outdent: bool,
) -> Result<Vec<EditRecord>, EditError> {
    let (first, last) = line_range(buf, first, last)?;
    let unit = style.unit();
    let mut out = Vec::new();
    for line in (first..=last).rev() {
        let text = buf.line_text(line).ok_or(EditError::OutOfRange)?;
        let start = buf.line_start(line)?;
        if outdent {
            let n = if text.starts_with('\t') {
                1
            } else {
                text.chars()
                    .take(style.width().max(1))
                    .take_while(|&c| c == ' ')
                    .count()
            };
            if n > 0 {
                out.push(buf.remove(start..start + n)?);
            }
        } else if !text.is_empty() {
            out.push(buf.insert(start, &unit)?);
        }
    }
    Ok(out)
}

/// Inserts a copy of lines `first..=last` below them.
pub fn duplicate_lines(
    buf: &mut Buffer,
    first: usize,
    last: usize,
) -> Result<EditRecord, EditError> {
    let (first, last) = line_range(buf, first, last)?;
    let start = buf.line_start(first)?;
    let end = buf.line_start(last + 1).unwrap_or(buf.len_chars());
    let mut text = buf.slice(start..end)?;
    if buf.line_ending(last) == Some("") {
        // The last line has no ending: the copy needs one in front.
        text.insert(0, '\n');
    }
    buf.insert(end, &text)
}

/// Removes lines `first..=last` with their endings.
pub fn delete_lines(buf: &mut Buffer, first: usize, last: usize) -> Result<EditRecord, EditError> {
    let (first, last) = line_range(buf, first, last)?;
    let mut start = buf.line_start(first)?;
    let end = buf.line_start(last + 1).unwrap_or(buf.len_chars());
    // Removing the final line takes the ending of the line before it too.
    if end == buf.len_chars() && buf.line_ending(last) == Some("") && start > 0 {
        start -= 1;
        if buf.char_at(start.saturating_sub(1)) == Some('\r') && start > 0 {
            start -= 1;
        }
    }
    buf.remove(start..end)
}

/// Moves lines `first..=last` one line up or down. `None` at the edge of the buffer.
pub fn move_lines(
    buf: &mut Buffer,
    first: usize,
    last: usize,
    down: bool,
) -> Result<Option<EditRecord>, EditError> {
    let (first, last) = line_range(buf, first, last)?;
    let (lo, hi) = if down {
        if last + 1 >= buf.len_lines() {
            return Ok(None);
        }
        (first, last + 1)
    } else {
        if first == 0 {
            return Ok(None);
        }
        (first - 1, last)
    };
    let mut contents = Vec::new();
    let mut endings = Vec::new();
    for line in lo..=hi {
        contents.push(buf.line_text(line).ok_or(EditError::OutOfRange)?);
        endings.push(buf.line_ending(line).ok_or(EditError::OutOfRange)?);
    }
    // One line (the neighbour) hops over the block; line endings stay where they are,
    // so a last line without an ending keeps being the last.
    if down {
        if let Some(neighbour) = contents.pop() {
            contents.insert(0, neighbour);
        }
    } else if !contents.is_empty() {
        let neighbour = contents.remove(0);
        contents.push(neighbour);
    }
    let mut text = String::new();
    for (c, e) in contents.iter().zip(&endings) {
        text.push_str(c);
        text.push_str(e);
    }
    let start = buf.line_start(lo)?;
    let end = buf.line_start(hi + 1).unwrap_or(buf.len_chars());
    buf.replace(start..end, &text).map(Some)
}

/// Comments or uncomments lines `first..=last` with `prefix` (such as `//`): when every
/// non-blank line already has it, it is removed, otherwise it is added at the shallowest
/// indent.
pub fn toggle_comment(
    buf: &mut Buffer,
    first: usize,
    last: usize,
    prefix: &str,
) -> Result<Vec<EditRecord>, EditError> {
    let (first, last) = line_range(buf, first, last)?;
    let mut lines = Vec::new();
    for line in first..=last {
        lines.push((line, buf.line_text(line).ok_or(EditError::OutOfRange)?));
    }
    let code: Vec<_> = lines.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
    if code.is_empty() || prefix.is_empty() {
        return Ok(Vec::new());
    }
    let all_commented = code.iter().all(|(_, t)| t.trim_start().starts_with(prefix));
    let mut out = Vec::new();
    if all_commented {
        for (line, text) in code.iter().rev() {
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let rest: String = text.chars().skip(indent + prefix.chars().count()).collect();
            let strip = prefix.chars().count() + usize::from(rest.starts_with(' '));
            let at = buf.line_start(*line)? + indent;
            out.push(buf.remove(at..at + strip)?);
        }
    } else {
        let col = code
            .iter()
            .map(|(_, t)| t.chars().take_while(|c| c.is_whitespace()).count())
            .min()
            .unwrap_or(0);
        let text = format!("{prefix} ");
        for (line, _) in code.iter().rev() {
            let at = buf.line_start(*line)? + col;
            out.push(buf.insert(at, &text)?);
        }
    }
    Ok(out)
}

fn partner(c: char) -> Option<(char, bool)> {
    match c {
        '(' => Some((')', true)),
        '[' => Some((']', true)),
        '{' => Some(('}', true)),
        ')' => Some(('(', false)),
        ']' => Some(('[', false)),
        '}' => Some(('{', false)),
        _ => None,
    }
}

/// For a caret at char `at`, the bracket next to it (after, else before) and its partner,
/// as char indexes. Scans at most `limit` chars, so it stays cheap in huge files. Strings
/// and comments are not skipped here; the syntax-aware version lives with the highlighter.
pub fn match_bracket(buf: &Buffer, at: usize, limit: usize) -> Option<(usize, usize)> {
    let candidates = [Some(at), at.checked_sub(1)];
    for idx in candidates.into_iter().flatten() {
        let Some(c) = buf.char_at(idx) else {
            continue;
        };
        let Some((want, forward)) = partner(c) else {
            continue;
        };
        let mut depth = 0usize;
        if forward {
            for (n, ch) in buf.chars_from(idx + 1)?.enumerate().take(limit) {
                if ch == c {
                    depth += 1;
                } else if ch == want {
                    if depth == 0 {
                        return Some((idx, idx + 1 + n));
                    }
                    depth -= 1;
                }
            }
        } else {
            for (n, ch) in buf.chars_before(idx)?.enumerate().take(limit) {
                if ch == c {
                    depth += 1;
                } else if ch == want {
                    if depth == 0 {
                        return Some((idx, idx - 1 - n));
                    }
                    depth -= 1;
                }
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SP4: IndentStyle = IndentStyle::Spaces(4);

    #[test]
    fn enter_keeps_indent_and_deepens_after_openers() {
        let b = Buffer::from_text("    if x:");
        assert_eq!(
            newline_text(&b, 9, SP4, Eol::Lf, true).as_deref(),
            Ok("\n        ")
        );
        assert_eq!(
            newline_text(&b, 9, SP4, Eol::Lf, false).as_deref(),
            Ok("\n    ")
        );
        let b = Buffer::from_text("\tfn f() {");
        assert_eq!(
            newline_text(&b, 9, IndentStyle::Tab, Eol::CrLf, false).as_deref(),
            Ok("\r\n\t\t")
        );
        // In the middle of the indent, only the part before the caret counts.
        let b = Buffer::from_text("        x");
        assert_eq!(
            newline_text(&b, 4, SP4, Eol::Lf, false).as_deref(),
            Ok("\n    ")
        );
        assert!(newline_text(&b, 99, SP4, Eol::Lf, false).is_err());
    }

    #[test]
    fn indent_detection() {
        assert_eq!(
            detect_indent(&Buffer::from_text("a\n  b\n    c\n")),
            IndentStyle::Spaces(2)
        );
        assert_eq!(
            detect_indent(&Buffer::from_text("a\n\tb\n\t\tc\n")),
            IndentStyle::Tab
        );
        assert_eq!(
            detect_indent(&Buffer::from_text("a\nb\n")),
            IndentStyle::Spaces(4)
        );
    }

    #[test]
    fn indent_and_outdent_a_block() {
        let mut b = Buffer::from_text("a\n\nb\nc");
        let recs = indent_lines(&mut b, 0, 2, SP4, false).expect("indent");
        assert_eq!(recs.len(), 2);
        assert_eq!(b.to_text(), "    a\n\n    b\nc");
        indent_lines(&mut b, 0, 3, SP4, true).expect("outdent");
        assert_eq!(b.to_text(), "a\n\nb\nc");
        let mut t = Buffer::from_text("\ta\n  b");
        indent_lines(&mut t, 0, 1, SP4, true).expect("outdent");
        assert_eq!(t.to_text(), "a\nb");
    }

    #[test]
    fn duplicate_and_delete_lines() {
        let mut b = Buffer::from_text("a\nb\nc");
        duplicate_lines(&mut b, 1, 2).expect("dup");
        assert_eq!(b.to_text(), "a\nb\nc\nb\nc");
        delete_lines(&mut b, 3, 4).expect("delete tail");
        assert_eq!(b.to_text(), "a\nb\nc");
        delete_lines(&mut b, 0, 0).expect("delete head");
        assert_eq!(b.to_text(), "b\nc");
        let mut crlf = Buffer::from_text("a\r\nb\r\nc");
        delete_lines(&mut crlf, 2, 2).expect("delete last");
        assert_eq!(crlf.to_text(), "a\r\nb");
    }

    #[test]
    fn move_lines_up_and_down() {
        let mut b = Buffer::from_text("a\nb\nc\nd");
        move_lines(&mut b, 1, 2, true)
            .expect("down")
            .expect("moved");
        assert_eq!(b.to_text(), "a\nd\nb\nc");
        move_lines(&mut b, 2, 3, false).expect("up").expect("moved");
        assert_eq!(b.to_text(), "a\nb\nc\nd");
        assert_eq!(move_lines(&mut b, 0, 0, false), Ok(None));
        assert_eq!(move_lines(&mut b, 3, 3, true), Ok(None));
        // The last line has no ending; it keeps being last.
        let mut e = Buffer::from_text("x\ny");
        move_lines(&mut e, 0, 0, true)
            .expect("down")
            .expect("moved");
        assert_eq!(e.to_text(), "y\nx");
    }

    #[test]
    fn comment_toggle_round_trip() {
        let mut b = Buffer::from_text("fn a() {\n    x();\n\n    y();\n}");
        let recs = toggle_comment(&mut b, 0, 4, "//").expect("comment");
        assert_eq!(recs.len(), 4);
        assert_eq!(b.to_text(), "// fn a() {\n//     x();\n\n//     y();\n// }");
        toggle_comment(&mut b, 0, 4, "//").expect("uncomment");
        assert_eq!(b.to_text(), "fn a() {\n    x();\n\n    y();\n}");
        assert_eq!(
            toggle_comment(&mut Buffer::from_text("\n\n"), 0, 1, "#"),
            Ok(vec![])
        );
    }

    #[test]
    fn partially_commented_block_gets_fully_commented() {
        let mut b = Buffer::from_text("# a\nb");
        toggle_comment(&mut b, 0, 1, "#").expect("comment");
        assert_eq!(b.to_text(), "# # a\n# b");
    }

    #[test]
    fn brackets_match_both_ways_and_nest() {
        let b = Buffer::from_text("f(a, (b), [c])");
        assert_eq!(match_bracket(&b, 1, 1000), Some((1, 13)));
        assert_eq!(match_bracket(&b, 2, 1000), Some((1, 13)));
        assert_eq!(match_bracket(&b, 14, 1000), Some((13, 1)));
        assert_eq!(match_bracket(&b, 5, 1000), Some((5, 7)));
        assert_eq!(match_bracket(&b, 0, 1000), None);
        assert_eq!(match_bracket(&Buffer::from_text("(("), 0, 1000), None);
        // The scan limit keeps huge files cheap.
        let far = Buffer::from_text(&format!("({})", "x".repeat(5000)));
        assert_eq!(match_bracket(&far, 0, 100), None);
        assert_eq!(match_bracket(&far, 0, 10_000), Some((0, 5001)));
    }

    #[test]
    fn bad_line_ranges_are_errors() {
        let mut b = Buffer::from_text("a\nb");
        assert!(indent_lines(&mut b, 5, 6, SP4, false).is_err());
        assert!(delete_lines(&mut b, 2, 1).is_err());
        assert_eq!(b.to_text(), "a\nb");
    }
}
