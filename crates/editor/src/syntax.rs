//! Incremental syntax parsing and viewport-scoped highlighting on tree-sitter.
//!
//! The UI thread owns a [`Syntax`]: it records each edit (`note_edit`) and asks for the
//! spans of the visible lines (`highlight`, ~1 ms). Re-parsing runs elsewhere: `job`
//! snapshots the buffer (O(1) rope clone) into a [`ParseJob`] that a worker runs under a
//! time budget and a cancel flag; the finished tree comes back through `install`. While a
//! parse is pending the previous tree, with the edits applied, keeps colouring.

use std::collections::HashMap;
use std::fmt;
use std::ops::{ControlFlow, Range};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use streaming_iterator::StreamingIterator;
use tree_sitter::{InputEdit, Node, Parser, Point, Query, QueryCursor, Tree};

use crate::buffer::{Buffer, EditRecord};
use crate::lang::{Lang, grammar};

/// Files above this are not parsed (design.md, spike results).
pub const DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024;
pub const FULL_PARSE_BUDGET: Duration = Duration::from_secs(5);
pub const INCREMENTAL_BUDGET: Duration = Duration::from_millis(500);
/// Cap on captures collected for one viewport.
const MAX_SPANS: usize = 100_000;

/// Semantic token classes; the UI maps them to theme tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Keyword,
    String,
    Escape,
    Comment,
    Number,
    Constant,
    Function,
    Type,
    Property,
    Variable,
    Operator,
    Punctuation,
    Tag,
    Attribute,
    Heading,
    Emphasis,
    Link,
}

/// One coloured run inside a line: byte offsets relative to the line start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenSpan {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyntaxError {
    /// No grammar for this language in this build, or its query did not compile.
    Unsupported,
    Cancelled,
    TimedOut,
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "syntax colouring is not available for this file",
            Self::Cancelled => "parsing was cancelled",
            Self::TimedOut => "parsing took too long; showing plain text",
        })
    }
}

impl std::error::Error for SyntaxError {}

/// Cancel flag shared with the UI; the time budget is set per job.
#[derive(Clone, Debug, Default)]
pub struct ParseCtl {
    pub cancel: Arc<AtomicBool>,
}

struct Compiled {
    lang: tree_sitter::Language,
    query: Query,
    kinds: Vec<Option<Kind>>,
}

type Cache = Mutex<HashMap<Lang, Option<Arc<Compiled>>>>;

fn compiled(lang: Lang) -> Option<Arc<Compiled>> {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache
        .entry(lang)
        .or_insert_with(|| {
            let (language, source) = grammar(lang)?;
            let query = Query::new(&language, &source).ok()?;
            let kinds = query.capture_names().iter().map(|n| kind_of(n)).collect();
            Some(Arc::new(Compiled {
                lang: language,
                query,
                kinds,
            }))
        })
        .clone()
}

fn kind_of(name: &str) -> Option<Kind> {
    let mut parts = name.split('.');
    let head = parts.next()?;
    let sub = parts.next();
    Some(match (head, sub) {
        ("keyword" | "conditional" | "repeat" | "include" | "storageclass", _) => Kind::Keyword,
        ("string", Some("escape")) | ("escape", _) => Kind::Escape,
        ("string", Some("special")) if name.contains("url") => Kind::Link,
        ("string" | "character", _) => Kind::String,
        ("comment", _) => Kind::Comment,
        ("number" | "float", _) | ("constant", Some("numeric")) => Kind::Number,
        ("constant" | "boolean", _) | ("variable", Some("builtin")) => Kind::Constant,
        ("function" | "method" | "constructor", _) => Kind::Function,
        ("type" | "module" | "namespace", _) => Kind::Type,
        ("property" | "field" | "label", _) => Kind::Property,
        ("variable", Some("parameter")) | ("parameter", _) => Kind::Variable,
        ("attribute", _) => Kind::Attribute,
        ("operator", _) => Kind::Operator,
        ("punctuation" | "delimiter", _) => Kind::Punctuation,
        ("tag", _) => Kind::Tag,
        ("markup", Some("heading")) | ("text", Some("title")) => Kind::Heading,
        ("markup", Some("italic" | "bold" | "strong")) | ("text", Some("emphasis" | "strong")) => {
            Kind::Emphasis
        }
        ("markup", Some("link")) | ("text", Some("uri" | "reference")) => Kind::Link,
        _ => return None,
    })
}

/// A re-parse for a worker thread.
pub struct ParseJob {
    lang: Lang,
    old: Option<Tree>,
    buf: Buffer,
    version: u64,
    budget: Duration,
}

impl ParseJob {
    /// The buffer version this job parses.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Blocking; stops at the budget or when `ctl.cancel` is set.
    pub fn run(self, ctl: &ParseCtl) -> Result<(Tree, u64), SyntaxError> {
        let compiled = compiled(self.lang).ok_or(SyntaxError::Unsupported)?;
        let mut parser = Parser::new();
        parser
            .set_language(&compiled.lang)
            .map_err(|_| SyntaxError::Unsupported)?;
        let deadline = Instant::now() + self.budget;
        let buf = &self.buf;
        let mut timed_out = false;
        let tree = {
            let mut input = |byte: usize, _: Point| buf.text_from_byte(byte);
            let mut stop = |_: &tree_sitter::ParseState| {
                if ctl.cancel.load(Ordering::Relaxed) {
                    ControlFlow::Break(())
                } else if Instant::now() >= deadline {
                    timed_out = true;
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let options = tree_sitter::ParseOptions::new().progress_callback(&mut stop);
            parser.parse_with_options(&mut input, self.old.as_ref(), Some(options))
        };
        match tree {
            Some(t) => Ok((t, self.version)),
            None if timed_out => Err(SyntaxError::TimedOut),
            None => Err(SyntaxError::Cancelled),
        }
    }
}

/// Parse state of one buffer.
pub struct Syntax {
    lang: Lang,
    tree: Option<Tree>,
    /// Buffer version the tree was parsed from (before later edits were noted).
    parsed_version: Option<u64>,
}

impl Syntax {
    pub fn new(lang: Lang) -> Result<Self, SyntaxError> {
        if !lang.available() {
            return Err(SyntaxError::Unsupported);
        }
        Ok(Self {
            lang,
            tree: None,
            parsed_version: None,
        })
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    pub fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// Whether the installed tree was parsed from this buffer version.
    pub fn is_current(&self, buf: &Buffer) -> bool {
        self.lang == Lang::Sql || self.parsed_version == Some(buf.version())
    }

    /// Records an edit that was just applied to `buf`, shifting the tree's ranges.
    pub fn note_edit(&mut self, buf: &Buffer, rec: &EditRecord) {
        if let (Some(tree), Some(edit)) = (self.tree.as_mut(), input_edit(buf, rec)) {
            tree.edit(&edit);
        }
    }

    /// A job that parses the buffer as it is now, reusing the (edited) previous tree.
    pub fn job(&self, buf: &Buffer, budget: Option<Duration>) -> ParseJob {
        ParseJob {
            lang: self.lang,
            old: self.tree.clone(),
            buf: buf.clone(),
            version: buf.version(),
            budget: budget.unwrap_or(if self.tree.is_some() {
                INCREMENTAL_BUDGET
            } else {
                FULL_PARSE_BUDGET
            }),
        }
    }

    /// Installs a finished parse, unless a newer one is already in.
    pub fn install(&mut self, tree: Tree, version: u64) {
        if self.parsed_version.is_none_or(|v| v <= version) {
            self.tree = Some(tree);
            self.parsed_version = Some(version);
        }
    }

    /// Parses on the calling thread (tests; the app parses on a worker with [`Self::job`]).
    #[cfg(test)]
    pub(crate) fn parse_now(&mut self, buf: &Buffer, ctl: &ParseCtl) -> Result<(), SyntaxError> {
        if self.lang == Lang::Sql {
            return Ok(());
        }
        let (tree, version) = self.job(buf, None).run(ctl)?;
        self.install(tree, version);
        Ok(())
    }

    /// Bracket depths for `lines` (rainbow brackets), one `Vec` per line; empty without
    /// a tree.
    pub fn rainbow(&self, buf: &Buffer, lines: Range<usize>) -> Vec<Vec<crate::BracketSpan>> {
        match &self.tree {
            Some(tree) if self.lang != Lang::Sql => crate::brackets::rainbow(tree, buf, lines),
            _ => Vec::new(),
        }
    }

    /// Spans for `lines` (first included, last excluded), one `Vec` per line.
    pub fn highlight(&self, buf: &Buffer, lines: Range<usize>) -> Vec<Vec<TokenSpan>> {
        let last = lines.end.min(buf.len_lines());
        let first = lines.start.min(last);
        let count = last - first;
        if self.lang == Lang::Sql {
            let text: Vec<String> = (first..last)
                .map(|l| buf.line_text(l).unwrap_or_default())
                .collect();
            return crate::sql::lex(&text);
        }
        let mut out = vec![Vec::new(); count];
        let (Some(tree), Some(compiled)) = (self.tree.as_ref(), compiled(self.lang)) else {
            return out;
        };
        // Byte offset where each line starts, plus the end of the last line.
        let mut starts = Vec::with_capacity(count + 1);
        for l in first..=last {
            let b = buf
                .line_start(l)
                .and_then(|c| buf.char_to_byte(c))
                .unwrap_or_else(|_| buf.len_bytes());
            starts.push(b);
        }
        let (Some(&from), Some(&to)) = (starts.first(), starts.last()) else {
            return out;
        };
        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(from..to);
        let root: Node = tree.root_node();
        let mut caps: Vec<(usize, usize, usize, Kind)> = Vec::new();
        let mut it = cursor.captures(&compiled.query, root, |n: Node| {
            buf.byte_chunks(n.byte_range()).map(str::as_bytes)
        });
        while let Some((m, idx)) = it.next() {
            let Some(c) = m.captures().get(*idx) else {
                continue;
            };
            let Some(Some(kind)) = compiled.kinds.get(c.index as usize) else {
                continue;
            };
            let r = c.node.byte_range();
            let (s, e) = (r.start.max(from), r.end.min(to));
            if s < e {
                caps.push((s, e, m.pattern_index, *kind));
            }
            if caps.len() >= MAX_SPANS {
                break;
            }
        }
        for (s, e, kind) in flatten(caps) {
            split_into_lines(&starts, s, e, kind, &mut out);
        }
        out
    }
}

fn input_edit(buf: &Buffer, rec: &EditRecord) -> Option<InputEdit> {
    let line_start_byte = buf
        .line_start(rec.start.line)
        .and_then(|c| buf.char_to_byte(c))
        .ok()?;
    let start_col = rec.start_byte.checked_sub(line_start_byte)?;
    let end_point = |text: &str| match text.rsplit_once('\n') {
        Some((before, last)) => Point {
            row: rec.start.line + before.matches('\n').count() + 1,
            column: last.len(),
        },
        None => Point {
            row: rec.start.line,
            column: start_col + text.len(),
        },
    };
    Some(InputEdit {
        start_byte: rec.start_byte,
        old_end_byte: rec.old_end_byte,
        new_end_byte: rec.new_end_byte,
        start_position: Point {
            row: rec.start.line,
            column: start_col,
        },
        old_end_position: end_point(&rec.removed),
        new_end_position: end_point(&rec.inserted),
    })
}

/// Resolves overlapping captures: at every position the match from the later pattern
/// wins (the usual highlight-query convention). Adjacent equal runs are merged.
fn flatten(mut caps: Vec<(usize, usize, usize, Kind)>) -> Vec<(usize, usize, Kind)> {
    caps.sort_by_key(|c| c.0);
    let mut points: Vec<usize> = caps.iter().flat_map(|c| [c.0, c.1]).collect();
    points.sort_unstable();
    points.dedup();
    let mut out: Vec<(usize, usize, Kind)> = Vec::new();
    let mut active: Vec<(usize, usize, usize, Kind)> = Vec::new();
    let mut next = 0;
    for w in points.windows(2) {
        let &[a, b] = w else { continue };
        while let Some(c) = caps.get(next).filter(|c| c.0 <= a) {
            active.push(*c);
            next += 1;
        }
        active.retain(|c| c.1 > a);
        let Some(best) = active.iter().max_by_key(|c| c.2) else {
            continue;
        };
        match out.last_mut() {
            Some(last) if last.1 == a && last.2 == best.3 => last.1 = b,
            _ => out.push((a, b, best.3)),
        }
    }
    out
}

fn split_into_lines(starts: &[usize], s: usize, e: usize, kind: Kind, out: &mut [Vec<TokenSpan>]) {
    let mut line = starts.partition_point(|&b| b <= s).saturating_sub(1);
    while let (Some(&ls), Some(&next)) = (starts.get(line), starts.get(line + 1)) {
        if ls >= e {
            break;
        }
        // A span may cover the line terminator; clipping to the text is the UI's concern.
        let (a, b) = (s.max(ls), e.min(next));
        if a < b
            && let Some(spans) = out.get_mut(line)
        {
            spans.push(TokenSpan {
                start: a - ls,
                end: b - ls,
                kind,
            });
        }
        line += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(spans: &[TokenSpan], line: &str, kind: Kind) -> Vec<String> {
        spans
            .iter()
            .filter(|s| s.kind == kind)
            .filter_map(|s| line.get(s.start..s.end.min(line.len())))
            .map(str::to_string)
            .collect()
    }

    fn colour(lang: Lang, src: &str) -> (Buffer, Syntax, Vec<Vec<TokenSpan>>) {
        let b = Buffer::from_text(src);
        let mut s = Syntax::new(lang).expect("grammar");
        s.parse_now(&b, &ParseCtl::default()).expect("parse");
        let h = s.highlight(&b, 0..b.len_lines());
        (b, s, h)
    }

    #[test]
    fn rust_tokens() {
        let src = "fn main() {\n    let s = \"hi\"; // note\n    let n = 42;\n}\n";
        let (_, _, h) = colour(Lang::Rust, src);
        assert!(lit(&h[0], "fn main() {", Kind::Keyword).contains(&"fn".to_string()));
        let l1 = "    let s = \"hi\"; // note";
        assert_eq!(lit(&h[1], l1, Kind::String), vec!["\"hi\""]);
        assert_eq!(lit(&h[1], l1, Kind::Comment), vec!["// note"]);
        // The Rust query files numeric literals under `constant.builtin`.
        assert_eq!(lit(&h[2], "    let n = 42;", Kind::Constant), vec!["42"]);
    }

    #[test]
    fn every_language_highlights_a_sample() {
        let samples: &[(Lang, &str)] = &[
            (Lang::Toml, "[a]\nname = \"x\"\nn = 3\n"),
            (Lang::Json, "{\"a\": [1, true, null], \"b\": \"c\"}\n"),
            (Lang::Yaml, "a: 1\nb: \"x\"\n# c\n"),
            (Lang::Markdown, "# Title\n\nsome *text*\n"),
            (Lang::Bash, "if [ -f x ]; then echo \"hi\"; fi # c\n"),
            (Lang::Python, "def f(x):\n    return \"s\" # c\n"),
            (Lang::JavaScript, "function f() { return \"s\"; } // c\n"),
            (Lang::TypeScript, "const x: number = 1; // c\n"),
            (Lang::Tsx, "const a = <div id=\"x\">hi</div>;\n"),
            (Lang::C, "int main(void) { return 0; } // c\n"),
            (Lang::Cpp, "class A { public: int f(); }; // c\n"),
            (
                Lang::Go,
                "package main\nfunc f() string { return \"s\" } // c\n",
            ),
            (Lang::Java, "class A { String s = \"x\"; } // c\n"),
            (Lang::Html, "<div class=\"a\">hi</div><!-- c -->\n"),
            (Lang::Css, "a { color: red; } /* c */\n"),
            (Lang::Xml, "<a b=\"c\">t</a><!-- c -->\n"),
            (Lang::Php, "<?php function f() { return \"s\"; } // c\n"),
            (Lang::Sql, "SELECT a, 'x' FROM t WHERE n > 3 -- c\n"),
        ];
        for (lang, src) in samples {
            let (_, _, h) = colour(*lang, src);
            let total: usize = h.iter().map(Vec::len).sum();
            assert!(total > 0, "{} produced no tokens", lang.label());
        }
    }

    #[test]
    fn incremental_matches_full_reparse() {
        let mut b = Buffer::from_text("fn a() {\n    let x = 1;\n}\n");
        let mut s = Syntax::new(Lang::Rust).expect("grammar");
        let ctl = ParseCtl::default();
        s.parse_now(&b, &ctl).expect("parse");
        for (at, text) in [(21, "\"str\""), (3, "bc"), (0, "// c\n")] {
            let rec = b.insert(at, text).expect("edit");
            s.note_edit(&b, &rec);
            s.parse_now(&b, &ctl).expect("reparse");
        }
        let rec = b.remove(0..3).expect("edit");
        s.note_edit(&b, &rec);
        s.parse_now(&b, &ctl).expect("reparse");
        let mut fresh = Syntax::new(Lang::Rust).expect("grammar");
        fresh.parse_now(&b, &ctl).expect("parse");
        assert_eq!(
            s.highlight(&b, 0..b.len_lines()),
            fresh.highlight(&b, 0..b.len_lines())
        );
        assert!(s.is_current(&b));
    }

    #[test]
    fn stale_tree_keeps_colouring_after_an_edit() {
        let mut b = Buffer::from_text("let a = 1;\nlet b = \"x\";\n");
        let mut s = Syntax::new(Lang::Rust).expect("grammar");
        s.parse_now(&b, &ParseCtl::default()).expect("parse");
        let rec = b.insert(0, "\n\n").expect("edit");
        s.note_edit(&b, &rec);
        assert!(!s.is_current(&b));
        let h = s.highlight(&b, 0..b.len_lines());
        assert!(
            h.get(3)
                .is_some_and(|l| l.iter().any(|t| t.kind == Kind::String))
        );
    }

    #[test]
    fn cancel_and_budget_are_honoured() {
        let big = "fn f() { let x = (1 + 2) * 3; }\n".repeat(200_000);
        let b = Buffer::from_text(&big);
        let s = Syntax::new(Lang::Rust).expect("grammar");
        let ctl = ParseCtl::default();
        assert_eq!(
            s.job(&b, Some(Duration::from_millis(1))).run(&ctl).err(),
            Some(SyntaxError::TimedOut)
        );
        ctl.cancel.store(true, Ordering::Relaxed);
        assert_eq!(
            s.job(&b, Some(Duration::from_secs(60))).run(&ctl).err(),
            Some(SyntaxError::Cancelled)
        );
    }

    #[test]
    fn hostile_input_does_not_panic() {
        let nested = format!("{}{}", "(".repeat(50_000), ")".repeat(50_000));
        let junk = "\u{0}\u{1f}}{{[[<<'\"`/*\n".repeat(2000);
        for lang in [
            Lang::Rust,
            Lang::Json,
            Lang::Python,
            Lang::Html,
            Lang::Sql,
            Lang::Php,
        ] {
            for src in [nested.as_str(), junk.as_str(), ""] {
                let b = Buffer::from_text(src);
                let mut s = Syntax::new(lang).expect("grammar");
                let _ = s.parse_now(&b, &ParseCtl::default());
                let h = s.highlight(&b, 0..b.len_lines() + 5);
                assert_eq!(h.len(), b.len_lines());
                let (hi, lo) = (9, 3);
                let _ = s.highlight(&b, hi..lo);
            }
        }
    }

    #[test]
    fn later_pattern_wins_and_spans_split_by_line() {
        let caps = vec![
            (0, 10, 0, Kind::String),
            (4, 6, 1, Kind::Escape),
            (10, 12, 0, Kind::String),
        ];
        assert_eq!(
            flatten(caps),
            vec![
                (0, 4, Kind::String),
                (4, 6, Kind::Escape),
                (6, 12, Kind::String)
            ]
        );
        let mut out = vec![Vec::new(); 3];
        split_into_lines(&[0, 5, 9, 14], 3, 11, Kind::Comment, &mut out);
        let span = |start, end| {
            vec![TokenSpan {
                start,
                end,
                kind: Kind::Comment,
            }]
        };
        assert_eq!(out[0], span(3, 5));
        assert_eq!(out[1], span(0, 4));
        assert_eq!(out[2], span(0, 2));
    }
}
