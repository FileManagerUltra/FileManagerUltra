//! Find and replace over a `Buffer`: literal, whole-word, case and regex modes. Matches
//! never span lines. Regexes are size-limited and run in linear time; long searches
//! check a cancel flag and a deadline between lines.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use regex::{Regex, RegexBuilder};

use crate::buffer::{Buffer, EditError, EditRecord};

pub const MAX_QUERY_CHARS: usize = 1024;
pub const MAX_REPLACEMENT_CHARS: usize = 64 * 1024;
/// Most matches `find_all` returns or `replace_all` rewrites in one go.
pub const MAX_MATCHES: usize = 1_000_000;
const REGEX_SIZE_LIMIT: usize = 1 << 20;
/// Lines scanned between cancel/deadline checks.
const CHECK_EVERY: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FindError {
    Empty,
    TooLong,
    BadPattern(String),
    TooManyMatches,
    Cancelled,
    TimedOut,
    Edit(EditError),
}

impl fmt::Display for FindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("nothing to search for"),
            Self::TooLong => f.write_str("the search text or replacement is too long"),
            Self::BadPattern(why) => write!(f, "invalid pattern: {why}"),
            Self::TooManyMatches => f.write_str("too many matches"),
            Self::Cancelled => f.write_str("search cancelled"),
            Self::TimedOut => f.write_str("search took too long"),
            Self::Edit(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for FindError {}

impl From<EditError> for FindError {
    fn from(e: EditError) -> Self {
        Self::Edit(e)
    }
}

/// Cancellation and time budget for a search.
#[derive(Clone, Copy, Default)]
pub struct SearchCtl<'a> {
    pub cancel: Option<&'a AtomicBool>,
    pub deadline: Option<Instant>,
}

impl SearchCtl<'_> {
    fn check(&self) -> Result<(), FindError> {
        if self.cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(FindError::Cancelled);
        }
        if self.deadline.is_some_and(|d| Instant::now() >= d) {
            return Err(FindError::TimedOut);
        }
        Ok(())
    }
}

/// A match as a char range of the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct Finder {
    re: Regex,
    regex_mode: bool,
}

impl Finder {
    pub fn new(query: &str, opts: FindOptions) -> Result<Self, FindError> {
        if query.is_empty() {
            return Err(FindError::Empty);
        }
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(FindError::TooLong);
        }
        let body = if opts.regex {
            query.to_string()
        } else {
            regex::escape(query)
        };
        let pattern = if opts.whole_word {
            format!(r"\b(?:{body})\b")
        } else {
            body
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!opts.case_sensitive)
            .multi_line(true)
            .size_limit(REGEX_SIZE_LIMIT)
            .dfa_size_limit(REGEX_SIZE_LIMIT * 8)
            .build()
            .map_err(|e| FindError::BadPattern(first_line(&e.to_string())))?;
        Ok(Self {
            re,
            regex_mode: opts.regex,
        })
    }

    /// The first match starting at or after `from`, wrapping around the end.
    pub fn find_next(
        &self,
        buf: &Buffer,
        from: usize,
        ctl: &SearchCtl<'_>,
    ) -> Result<Option<Match>, FindError> {
        let from = from.min(buf.len_chars());
        let first = buf.char_to_pos(from)?.line;
        let lines = buf.len_lines();
        let mut scanned = 0usize;
        // Rest of the start line and everything below, then the top down to the start line.
        for line in (first..lines).chain(0..=first) {
            scanned += 1;
            if scanned.is_multiple_of(CHECK_EVERY) {
                ctl.check()?;
            }
            let wrapped = scanned > lines.saturating_sub(first);
            if let Some(m) = self
                .line_matches(buf, line)?
                .into_iter()
                .find(|m| wrapped || m.start >= from)
            {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// The last match starting before `before`, wrapping around the start.
    pub fn find_prev(
        &self,
        buf: &Buffer,
        before: usize,
        ctl: &SearchCtl<'_>,
    ) -> Result<Option<Match>, FindError> {
        let before = before.min(buf.len_chars());
        let first = buf.char_to_pos(before)?.line;
        let lines = buf.len_lines();
        let mut scanned = 0usize;
        for line in (0..=first).rev().chain((first..lines).rev()) {
            scanned += 1;
            if scanned.is_multiple_of(CHECK_EVERY) {
                ctl.check()?;
            }
            let wrapped = scanned > first + 1;
            if let Some(m) = self
                .line_matches(buf, line)?
                .into_iter()
                .rev()
                .find(|m| wrapped || m.start < before)
            {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    /// The replacement for one match whose text is `found` (regex mode expands
    /// `$1`/`${name}`), or `None` when `found` is not a whole match.
    pub fn expand(&self, found: &str, replacement: &str) -> Option<String> {
        if replacement.chars().count() > MAX_REPLACEMENT_CHARS {
            return None;
        }
        let caps = self.re.captures(found)?;
        let whole = caps.get(0)?;
        if whole.start() != 0 || whole.end() != found.len() {
            return None;
        }
        let mut out = String::new();
        if self.regex_mode {
            caps.expand(replacement, &mut out);
        } else {
            out.push_str(replacement);
        }
        Some(out)
    }

    /// Every match in order, and whether the `limit` cut the list short.
    pub fn find_all(
        &self,
        buf: &Buffer,
        limit: usize,
        ctl: &SearchCtl<'_>,
    ) -> Result<(Vec<Match>, bool), FindError> {
        let limit = limit.min(MAX_MATCHES);
        let mut out = Vec::new();
        for line in 0..buf.len_lines() {
            if line % CHECK_EVERY == 0 {
                ctl.check()?;
            }
            for m in self.line_matches(buf, line)? {
                if out.len() >= limit {
                    return Ok((out, true));
                }
                out.push(m);
            }
        }
        Ok((out, false))
    }

    /// Replaces every match. In regex mode `$1`/`${name}` expand to captures. Returns the
    /// edits in the order applied (last match first), ready for `UndoLog::record_group`.
    pub fn replace_all(
        &self,
        buf: &mut Buffer,
        replacement: &str,
        ctl: &SearchCtl<'_>,
    ) -> Result<Vec<EditRecord>, FindError> {
        if replacement.chars().count() > MAX_REPLACEMENT_CHARS {
            return Err(FindError::TooLong);
        }
        let mut plan: Vec<(Match, String)> = Vec::new();
        for line in 0..buf.len_lines() {
            if line % CHECK_EVERY == 0 {
                ctl.check()?;
            }
            let Some(text) = buf.line_text(line) else {
                continue;
            };
            let line_start = buf.line_start(line)?;
            let mut chars_done = 0usize;
            let mut bytes_done = 0usize;
            for caps in self.re.captures_iter(&text) {
                let Some(m) = caps.get(0) else {
                    continue;
                };
                if m.start() == m.end() {
                    continue;
                }
                if plan.len() >= MAX_MATCHES {
                    return Err(FindError::TooManyMatches);
                }
                chars_done += text
                    .get(bytes_done..m.start())
                    .map_or(0, |s| s.chars().count());
                bytes_done = m.start();
                let start = line_start + chars_done;
                let end = start + m.as_str().chars().count();
                let mut out = String::new();
                if self.regex_mode {
                    caps.expand(replacement, &mut out);
                } else {
                    out.push_str(replacement);
                }
                plan.push((Match { start, end }, out));
            }
        }
        ctl.check()?;
        let mut records = Vec::with_capacity(plan.len());
        for (m, text) in plan.into_iter().rev() {
            records.push(buf.replace(m.start..m.end, &text)?);
        }
        Ok(records)
    }

    fn line_matches(&self, buf: &Buffer, line: usize) -> Result<Vec<Match>, FindError> {
        let Some(text) = buf.line_text(line) else {
            return Ok(Vec::new());
        };
        let line_start = buf.line_start(line)?;
        let mut out = Vec::new();
        let mut chars_done = 0usize;
        let mut bytes_done = 0usize;
        for m in self.re.find_iter(&text) {
            if m.start() == m.end() {
                continue;
            }
            chars_done += text
                .get(bytes_done..m.start())
                .map_or(0, |s| s.chars().count());
            bytes_done = m.start();
            let start = line_start + chars_done;
            out.push(Match {
                start,
                end: start + m.as_str().chars().count(),
            });
        }
        Ok(out)
    }
}

fn first_line(s: &str) -> String {
    s.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(s)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::{Coalesce, UndoLog};

    fn ctl() -> SearchCtl<'static> {
        SearchCtl::default()
    }

    fn finder(q: &str, case: bool, word: bool, re: bool) -> Finder {
        Finder::new(
            q,
            FindOptions {
                case_sensitive: case,
                whole_word: word,
                regex: re,
            },
        )
        .expect("finder")
    }

    #[test]
    fn next_and_prev_wrap_around() {
        let b = Buffer::from_text("foo bar\nfoo baz\nqux");
        let f = finder("foo", true, false, false);
        let m1 = f.find_next(&b, 0, &ctl()).expect("ok").expect("m");
        assert_eq!((m1.start, m1.end), (0, 3));
        let m2 = f.find_next(&b, 1, &ctl()).expect("ok").expect("m");
        assert_eq!(m2.start, 8);
        let wrap = f.find_next(&b, 9, &ctl()).expect("ok").expect("m");
        assert_eq!(wrap.start, 0);
        let p = f.find_prev(&b, 8, &ctl()).expect("ok").expect("m");
        assert_eq!(p.start, 0);
        let pw = f.find_prev(&b, 0, &ctl()).expect("ok").expect("m");
        assert_eq!(pw.start, 8);
    }

    #[test]
    fn case_and_whole_word() {
        let b = Buffer::from_text("Cat cat concat");
        assert_eq!(
            finder("cat", false, false, false)
                .find_all(&b, 100, &ctl())
                .expect("all")
                .0
                .len(),
            3
        );
        assert_eq!(
            finder("cat", true, false, false)
                .find_all(&b, 100, &ctl())
                .expect("all")
                .0
                .len(),
            2
        );
        let words = finder("cat", false, true, false)
            .find_all(&b, 100, &ctl())
            .expect("all")
            .0;
        assert_eq!(words.len(), 2);
    }

    #[test]
    fn positions_count_chars_not_bytes() {
        let b = Buffer::from_text("日本 x\né x");
        let all = finder("x", true, false, false)
            .find_all(&b, 10, &ctl())
            .expect("all")
            .0;
        assert_eq!(
            all,
            vec![Match { start: 3, end: 4 }, Match { start: 7, end: 8 }]
        );
        assert_eq!(b.slice(3..4).as_deref(), Ok("x"));
    }

    #[test]
    fn bad_and_oversized_patterns_are_errors() {
        let o = FindOptions {
            regex: true,
            ..Default::default()
        };
        assert!(matches!(Finder::new("(", o), Err(FindError::BadPattern(_))));
        assert_eq!(Finder::new("", o).err(), Some(FindError::Empty));
        let long = "a".repeat(MAX_QUERY_CHARS + 1);
        assert_eq!(Finder::new(&long, o).err(), Some(FindError::TooLong));
        // Blows past the compiled-size limit instead of eating memory.
        assert!(Finder::new(r"(\w{500}){500}", o).is_err());
    }

    #[test]
    fn empty_matches_do_not_loop() {
        let b = Buffer::from_text("abc\n\nabc");
        let f = finder("x*", true, false, true);
        assert_eq!(f.find_all(&b, 10, &ctl()).expect("all").0, vec![]);
        assert_eq!(f.find_next(&b, 0, &ctl()), Ok(None));
    }

    #[test]
    fn limit_truncates_and_cancel_stops() {
        let b = Buffer::from_text(&"a\n".repeat(1000));
        let f = finder("a", true, false, false);
        let (found, cut) = f.find_all(&b, 10, &ctl()).expect("all");
        assert_eq!((found.len(), cut), (10, true));
        let stop = AtomicBool::new(true);
        let c = SearchCtl {
            cancel: Some(&stop),
            deadline: None,
        };
        assert_eq!(f.find_all(&b, 10, &c).err(), Some(FindError::Cancelled));
        let late = SearchCtl {
            cancel: None,
            deadline: Some(Instant::now()),
        };
        assert_eq!(f.find_all(&b, 10, &late).err(), Some(FindError::TimedOut));
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let mut b = Buffer::from_text(&"foo bar\n".repeat(300));
        let before = b.to_text();
        let mut log = UndoLog::default();
        let f = finder("foo", true, false, false);
        let recs = f.replace_all(&mut b, "quux", &ctl()).expect("replace");
        assert_eq!(recs.len(), 300);
        assert!(b.to_text().starts_with("quux bar\nquux bar\n"));
        log.record_group(recs);
        assert_eq!(log.undo_steps(), 1);
        log.undo(&mut b).expect("undo");
        assert_eq!(b.to_text(), before);
        log.redo(&mut b).expect("redo");
        assert!(b.to_text().starts_with("quux bar\n"));
    }

    #[test]
    fn regex_replacement_expands_captures() {
        let mut b = Buffer::from_text("a=1\nb=22\n");
        let f = finder(r"(\w)=(\d+)", true, false, true);
        f.replace_all(&mut b, "$2:$1", &ctl()).expect("replace");
        assert_eq!(b.to_text(), "1:a\n22:b\n");
        let mut lit = Buffer::from_text("a=1\n");
        finder("a", true, false, false)
            .replace_all(&mut lit, "$1", &ctl())
            .expect("literal");
        assert_eq!(lit.to_text(), "$1=1\n");
    }

    #[test]
    fn replace_all_with_different_lengths_stays_correct() {
        let mut b = Buffer::from_text("ab ab ab");
        let f = finder("ab", true, false, false);
        let recs = f.replace_all(&mut b, "x", &ctl()).expect("replace");
        assert_eq!(b.to_text(), "x x x");
        let mut log = UndoLog::default();
        log.record_group(recs);
        let r = b.insert(0, "!").expect("insert");
        log.record(r, Coalesce::No);
        log.undo(&mut b).expect("undo");
        log.undo(&mut b).expect("undo");
        assert_eq!(b.to_text(), "ab ab ab");
    }

    #[test]
    fn expand_needs_a_whole_match() {
        let f = finder(r"(\w)=(\d+)", true, false, true);
        assert_eq!(f.expand("b=22", "$2:$1").as_deref(), Some("22:b"));
        assert_eq!(f.expand("xb=22", "$2"), None);
        let lit = finder("ab", false, false, false);
        assert_eq!(lit.expand("AB", "$1").as_deref(), Some("$1"));
    }
}
