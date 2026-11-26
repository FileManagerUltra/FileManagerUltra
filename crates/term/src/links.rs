//! Finding web links in the visible terminal screen. Screen text is untrusted: only
//! `http`/`https` URLs are recognised, and length is capped. A URL longer than the
//! screen is wide continues on the next rows when the terminal soft-wrapped it there
//! (not across real line breaks).

use crate::Snapshot;

const MAX_URL: usize = 2048;

/// The cells one row of a link covers; `start`/`end` are inclusive columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkSpan {
    pub row: u16,
    pub start: u16,
    pub end: u16,
}

/// A URL on the screen, one span per row it covers (in row order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub spans: Vec<LinkSpan>,
    pub url: String,
}

/// Most rows one wrapped line is followed over, either way.
const MAX_WRAPPED_ROWS: u16 = 64;

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
        url = url.strip_suffix(last).unwrap_or_default();
    }
}

/// The link under `(row, col)` of the snapshot, if any.
pub fn link_at(snap: &Snapshot, row: u16, col: u16) -> Option<Link> {
    if row >= snap.rows || col >= snap.cols {
        return None;
    }
    // The logical line: up while the row above wraps onto this one, down while this wraps.
    let mut first = row;
    while first > 0 && row - first < MAX_WRAPPED_ROWS && snap.row_wraps(first - 1) {
        first -= 1;
    }
    let mut last = row;
    while last + 1 < snap.rows && last - row < MAX_WRAPPED_ROWS && snap.row_wraps(last) {
        last += 1;
    }
    let mut line = String::new();
    // (byte offset of the cell's text, row, column)
    let mut starts: Vec<(usize, u16, u16)> = Vec::new();
    for r in first..=last {
        for c in 0..snap.cols {
            let cell = snap.cell(r, c)?;
            if cell.wide_continuation {
                continue;
            }
            starts.push((line.len(), r, c));
            if cell.text.is_empty() {
                line.push(' ');
            } else {
                line.push_str(&cell.text);
            }
        }
    }
    let clicked = starts
        .iter()
        .rev()
        .find(|(_, r, c)| *r == row && *c <= col)?
        .0;
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
                return Some(Link {
                    spans: spans(snap, &starts, begin, end),
                    url: url.to_string(),
                });
            }
        }
    }
    None
}

/// The cells of `begin..end` (byte offsets into the joined line), as one span per row.
/// A wide character's second cell is included with it.
fn spans(snap: &Snapshot, starts: &[(usize, u16, u16)], begin: usize, end: usize) -> Vec<LinkSpan> {
    let mut out: Vec<LinkSpan> = Vec::new();
    for (ix, &(byte, r, c)) in starts.iter().enumerate() {
        if byte < begin || byte >= end {
            continue;
        }
        // The cell runs up to the next cell's column on the same row (wide = 2 columns).
        let next = starts
            .get(ix + 1)
            .filter(|(_, nr, _)| *nr == r)
            .map_or(snap.cols, |(_, _, nc)| *nc);
        let cell_end = next.saturating_sub(1).max(c);
        match out.last_mut() {
            Some(span) if span.row == r => span.end = cell_end,
            _ => out.push(LinkSpan {
                row: r,
                start: c,
                end: cell_end,
            }),
        }
    }
    out
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
        assert_eq!(
            l.spans,
            vec![LinkSpan {
                row: 0,
                start: 4,
                end: 28
            }]
        );
    }

    #[test]
    fn trailing_multibyte_punctuation_is_handled() {
        assert_eq!(trim_end("https://a.org/x'"), "https://a.org/x");
        assert_eq!(trim_end("https://a.org/é)"), "https://a.org/é");
        assert_eq!(trim_end("https://a.org/ü"), "https://a.org/ü");
        assert_eq!(trim_end("..."), "");
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
        assert_eq!(
            l.spans,
            vec![LinkSpan {
                row: 0,
                start: 5,
                end: 17
            }]
        );
    }

    #[test]
    fn a_soft_wrapped_url_is_found_from_every_row() {
        // 60 columns: the URL starts at column 9 and wraps twice.
        let url = format!("https://example.com/{}", "a".repeat(100));
        let text = format!("remote:  {url} done");
        let s = snap(&text);
        assert!(s.row_wraps(0) && s.row_wraps(1) && !s.row_wraps(2));
        for (row, col) in [(0, 20), (1, 0), (1, 59), (2, 5)] {
            let l = link_at(&s, row, col).expect("link");
            assert_eq!(l.url, url, "from row {row}");
            assert_eq!(
                l.spans,
                vec![
                    LinkSpan {
                        row: 0,
                        start: 9,
                        end: 59
                    },
                    LinkSpan {
                        row: 1,
                        start: 0,
                        end: 59
                    },
                    LinkSpan {
                        row: 2,
                        start: 0,
                        end: 8
                    },
                ]
            );
        }
        assert_eq!(find(&text, 3), None, "before the link");
        assert!(link_at(&s, 2, 12).is_none(), "after the link");
    }

    #[test]
    fn a_hard_line_break_ends_the_url() {
        let s = snap("https://a.org/x\r\nmore");
        assert!(!s.row_wraps(0));
        assert_eq!(
            link_at(&s, 0, 3).map(|l| l.url).as_deref(),
            Some("https://a.org/x")
        );
        assert!(link_at(&s, 1, 1).is_none());
    }
}
