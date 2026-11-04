//! Finding web links in the visible terminal screen. Screen text is untrusted: only
//! `http`/`https` URLs on a single row are recognised, and length is capped.

use crate::Snapshot;

const MAX_URL: usize = 2048;

/// A URL on one screen row; `start`/`end` are inclusive columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub row: u16,
    pub start: u16,
    pub end: u16,
    pub url: String,
}

fn stops(c: char) -> bool {
    c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | '"' | '`' | '\u{2502}')
}

/// Drop trailing sentence punctuation and unbalanced closing brackets.
fn trim_end(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let open = match last {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        let drop = match open {
            Some(open) => url.matches(open).count() < url.matches(last).count(),
            None => matches!(last, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '*'),
        };
        if !drop {
            return url;
        }
        url = &url[..url.len() - last.len_utf8()];
    }
}

/// The link under `(row, col)` of the snapshot, if any.
pub fn link_at(snap: &Snapshot, row: u16, col: u16) -> Option<Link> {
    if row >= snap.rows || col >= snap.cols {
        return None;
    }
    let mut line = String::new();
    // (byte offset of the cell's text, column)
    let mut starts: Vec<(usize, u16)> = Vec::new();
    for c in 0..snap.cols {
        let cell = snap.cell(row, c)?;
        if cell.wide_continuation {
            continue;
        }
        starts.push((line.len(), c));
        if cell.text.is_empty() {
            line.push(' ');
        } else {
            line.push_str(&cell.text);
        }
    }
    let clicked = starts.iter().rev().find(|(_, c)| *c <= col)?.0;
    for scheme in ["https://", "http://"] {
        let mut from = 0;
        while let Some(found) = line.get(from..).and_then(|rest| rest.find(scheme)) {
            let begin = from + found;
            let tail = line.get(begin..)?;
            let stop = tail.find(stops).unwrap_or(tail.len());
            let url = trim_end(tail.get(..stop)?);
            let end = begin + url.len();
            from = begin + scheme.len();
            if url.len() <= scheme.len() || url.len() > MAX_URL {
                continue;
            }
            if clicked >= begin && clicked < end {
                let start_col = starts.iter().find(|(b, _)| *b == begin).map(|(_, c)| *c)?;
                let last_cell = starts
                    .iter()
                    .rev()
                    .find(|(b, _)| *b < end)
                    .map(|(_, c)| *c)?;
                let end_col = starts
                    .iter()
                    .find(|(b, _)| *b >= end)
                    .map_or(snap.cols, |(_, c)| *c)
                    .saturating_sub(1)
                    .max(last_cell);
                return Some(Link {
                    row,
                    start: start_col,
                    end: end_col,
                    url: url.to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Emulator;

    fn snap(text: &str) -> Snapshot {
        let mut e = Emulator::new(4, 60);
        e.feed(text.as_bytes());
        e.snapshot()
    }

    fn find(text: &str, col: u16) -> Option<String> {
        link_at(&snap(text), 0, col).map(|l| l.url)
    }

    #[test]
    fn finds_urls_and_trims_punctuation() {
        let line = "see https://example.com/a_(b), then";
        assert_eq!(find(line, 6).as_deref(), Some("https://example.com/a_(b)"));
        assert_eq!(find(line, 0), None);
        assert_eq!(find(line, 30), None);
        let l = link_at(&snap(line), 0, 10).expect("link");
        assert_eq!((l.start, l.end), (4, 28));
    }

    #[test]
    fn only_web_schemes_and_nothing_empty() {
        assert_eq!(find("ftp://x.org/y", 3), None);
        assert_eq!(find("file:///etc/passwd", 3), None);
        assert_eq!(find("javascript:alert(1) http://", 3), None);
        assert_eq!(find("http://", 3), None);
        assert_eq!(
            find("(http://a.org/x)", 3).as_deref(),
            Some("http://a.org/x")
        );
    }

    #[test]
    fn out_of_range_cells_and_wide_text_are_safe() {
        let s = snap("日本 https://a.org end");
        assert!(link_at(&s, 99, 0).is_none());
        assert!(link_at(&s, 0, 999).is_none());
        let l = link_at(&s, 0, 8).expect("link after wide chars");
        assert_eq!(l.url, "https://a.org");
        assert_eq!((l.start, l.end), (5, 17));
    }
}
