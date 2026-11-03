//! Bounded Markdown document model. HTML and image descriptions remain inert text;
//! links and images never cause resource loads.

use std::iter::Peekable;

use fm_vfs::{Entry, FmError, Vfs};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{Limits, Preview, Span, TextDoc, highlight, text};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeBlock {
    pub language: String,
    pub text: String,
    /// Highlighted on the preview worker, never during a UI frame.
    pub lines: Vec<Vec<Span>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub blocks: Vec<MdBlock>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub alignments: Vec<Alignment>,
    pub header: Vec<Vec<Inline>>,
    pub rows: Vec<Vec<Vec<Inline>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MdBlock {
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    Code(CodeBlock),
    List {
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Quote(Vec<MdBlock>),
    Table(Table),
    Rule,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MdDoc {
    pub blocks: Vec<MdBlock>,
    /// The Markdown text itself, highlighted, for the Source view.
    pub source: TextDoc,
    pub truncated: bool,
    pub total_bytes: u64,
}

const MAX_BYTES: usize = 1 << 20;
const MAX_BLOCKS: usize = 5_000;
const MAX_DEPTH: usize = 32;
const MAX_INLINES: usize = 30_000;
const MAX_CELLS: usize = 4_096;
const MAX_COLUMNS: usize = 64;

#[derive(Default)]
struct Budget {
    blocks: usize,
    inlines: usize,
    cells: usize,
    truncated: bool,
    exhausted: bool,
}

impl Budget {
    fn block(&mut self) -> bool {
        if self.blocks >= MAX_BLOCKS {
            self.stop();
            return false;
        }
        self.blocks += 1;
        true
    }

    fn stop(&mut self) {
        self.truncated = true;
        self.exhausted = true;
    }
}

type Events<'a> = Peekable<Parser<'a>>;

fn level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn inlines(events: &mut Events<'_>, budget: &mut Budget, task: &mut Option<bool>) -> Vec<Inline> {
    let mut out = Vec::new();
    let (mut bold, mut italic, mut strike) = (0usize, 0usize, 0usize);
    let mut link = None;
    while !budget.exhausted {
        let Some(event) = events.peek() else { break };
        let (text, code) = match event {
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => (t.to_string(), false),
            Event::Code(t) => (t.to_string(), true),
            Event::SoftBreak => (" ".into(), false),
            Event::HardBreak => ("\n".into(), false),
            Event::TaskListMarker(checked) => {
                *task = Some(*checked);
                events.next();
                continue;
            }
            Event::Start(Tag::Strong) => {
                bold += 1;
                events.next();
                continue;
            }
            Event::End(TagEnd::Strong) => {
                bold = bold.saturating_sub(1);
                events.next();
                continue;
            }
            Event::Start(Tag::Emphasis) => {
                italic += 1;
                events.next();
                continue;
            }
            Event::End(TagEnd::Emphasis) => {
                italic = italic.saturating_sub(1);
                events.next();
                continue;
            }
            Event::Start(Tag::Strikethrough) => {
                strike += 1;
                events.next();
                continue;
            }
            Event::End(TagEnd::Strikethrough) => {
                strike = strike.saturating_sub(1);
                events.next();
                continue;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                link = Some(dest_url.to_string());
                events.next();
                continue;
            }
            Event::End(TagEnd::Link) => {
                link = None;
                events.next();
                continue;
            }
            Event::Start(Tag::Image { .. }) => ("[Image: ".into(), false),
            Event::End(TagEnd::Image) => ("]".into(), false),
            _ => break,
        };
        if budget.inlines >= MAX_INLINES {
            budget.stop();
            break;
        }
        events.next();
        budget.inlines += 1;
        out.push(Inline {
            text,
            bold: bold > 0,
            italic: italic > 0,
            strike: strike > 0,
            code,
            link: link.clone(),
        });
    }
    out
}

fn list(events: &mut Events<'_>, budget: &mut Budget, depth: usize) -> Vec<ListItem> {
    let mut items = Vec::new();
    while !budget.exhausted {
        match events.next() {
            Some(Event::Start(Tag::Item)) => {
                if !budget.block() {
                    break;
                }
                let mut checked = None;
                let blocks = blocks(events, budget, depth, Some(TagEnd::Item), &mut checked);
                items.push(ListItem { checked, blocks });
            }
            Some(Event::End(TagEnd::List(_))) | None => break,
            _ => {}
        }
    }
    items
}

fn table(events: &mut Events<'_>, budget: &mut Budget, alignments: Vec<Alignment>) -> Table {
    let mut table = Table {
        alignments,
        header: Vec::new(),
        rows: Vec::new(),
    };
    let mut row = Vec::new();
    while !budget.exhausted {
        match events.next() {
            Some(Event::Start(Tag::TableCell)) => {
                if budget.cells >= MAX_CELLS || row.len() >= MAX_COLUMNS {
                    budget.stop();
                    break;
                }
                budget.cells += 1;
                row.push(inlines(events, budget, &mut None));
            }
            Some(Event::End(TagEnd::TableHead)) => table.header = std::mem::take(&mut row),
            Some(Event::End(TagEnd::TableRow)) => table.rows.push(std::mem::take(&mut row)),
            Some(Event::End(TagEnd::Table)) | None => break,
            _ => {}
        }
    }
    table
}

fn blocks(
    events: &mut Events<'_>,
    budget: &mut Budget,
    depth: usize,
    end: Option<TagEnd>,
    task: &mut Option<bool>,
) -> Vec<MdBlock> {
    let mut out = Vec::new();
    if depth >= MAX_DEPTH {
        budget.stop();
        return out;
    }
    while !budget.exhausted {
        if events.peek().is_none() {
            break;
        }
        if matches!(events.peek(), Some(Event::End(t)) if Some(*t) == end) {
            events.next();
            break;
        }
        // Tight list items have no Paragraph tags.
        if matches!(
            events.peek(),
            Some(
                Event::Text(_)
                    | Event::Code(_)
                    | Event::SoftBreak
                    | Event::HardBreak
                    | Event::InlineHtml(_)
                    | Event::TaskListMarker(_)
                    | Event::Start(
                        Tag::Strong
                            | Tag::Emphasis
                            | Tag::Strikethrough
                            | Tag::Link { .. }
                            | Tag::Image { .. }
                    )
            )
        ) {
            if !budget.block() {
                break;
            }
            out.push(MdBlock::Para(inlines(events, budget, task)));
            continue;
        }
        let Some(event) = events.next() else { break };
        let block = match event {
            Event::Start(Tag::Paragraph) => {
                if !budget.block() {
                    break;
                }
                MdBlock::Para(inlines(events, budget, task))
            }
            Event::Start(Tag::Heading { level: l, .. }) => {
                if !budget.block() {
                    break;
                }
                MdBlock::Heading(level(l), inlines(events, budget, task))
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                if !budget.block() {
                    break;
                }
                MdBlock::Quote(blocks(
                    events,
                    budget,
                    depth + 1,
                    Some(TagEnd::BlockQuote(kind)),
                    task,
                ))
            }
            Event::Start(Tag::List(start)) => {
                if !budget.block() {
                    break;
                }
                MdBlock::List {
                    start,
                    items: list(events, budget, depth + 1),
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                if !budget.block() {
                    break;
                }
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut text = String::new();
                for event in events.by_ref() {
                    match event {
                        Event::Text(t) => text.push_str(&t),
                        Event::End(TagEnd::CodeBlock) => break,
                        _ => {}
                    }
                }
                let text = text.trim_end_matches('\n').to_string();
                let lines = highlight::highlight(&text, &language, 20_000);
                MdBlock::Code(CodeBlock {
                    language,
                    text,
                    lines,
                })
            }
            Event::Start(Tag::Table(alignments)) => {
                if !budget.block() {
                    break;
                }
                if alignments.len() > MAX_COLUMNS {
                    budget.stop();
                    break;
                }
                let alignments = alignments
                    .into_iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::Center => Alignment::Center,
                        pulldown_cmark::Alignment::Right => Alignment::Right,
                        _ => Alignment::Left,
                    })
                    .collect();
                MdBlock::Table(table(events, budget, alignments))
            }
            Event::Html(t) => {
                if !budget.block() {
                    break;
                }
                MdBlock::Para(vec![Inline {
                    text: t.to_string(),
                    ..Default::default()
                }])
            }
            Event::Rule => {
                if !budget.block() {
                    break;
                }
                MdBlock::Rule
            }
            _ => continue,
        };
        out.push(block);
    }
    out
}

fn parse_document(src: &str, max_lines: usize, total_bytes: u64) -> MdDoc {
    let mut end = src.len().min(MAX_BYTES);
    while !src.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let src_capped = &src[..end];
    let line_end = src_capped
        .split_inclusive('\n')
        .take(max_lines)
        .map(str::len)
        .sum::<usize>();
    let mut budget = Budget::default();
    let mut events = Parser::new_ext(
        &src_capped[..line_end],
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS,
    )
    .peekable();
    let blocks = blocks(&mut events, &mut budget, 0, None, &mut None);
    let truncated = budget.truncated || line_end < src.len();
    MdDoc {
        blocks,
        source: TextDoc {
            lines: highlight::highlight(&src_capped[..line_end], "md", max_lines),
            lang: "md".into(),
            truncated,
            total_bytes,
        },
        truncated,
        total_bytes,
    }
}

pub fn parse(src: &str) -> Vec<MdBlock> {
    parse_document(src, 20_000, src.len() as u64).blocks
}

/// Plain rendered content, shared by clipboard export and document tests.
pub fn plain_text(blocks: &[MdBlock]) -> String {
    fn inline(parts: &[Inline]) -> String {
        parts.iter().map(|p| p.text.as_str()).collect()
    }
    fn collect(blocks: &[MdBlock], indent: usize, out: &mut Vec<String>) {
        for block in blocks {
            match block {
                MdBlock::Heading(_, parts) | MdBlock::Para(parts) => out.push(inline(parts)),
                MdBlock::Code(c) => out.push(c.text.clone()),
                MdBlock::Rule => out.push("---".into()),
                MdBlock::Quote(children) => collect(children, indent, out),
                MdBlock::List { start, items } => {
                    for (n, item) in items.iter().enumerate() {
                        let marker = match item.checked {
                            Some(true) => "[x]".into(),
                            Some(false) => "[ ]".into(),
                            None => start.map_or_else(
                                || "•".into(),
                                |start| format!("{}.", start.saturating_add(n as u64)),
                            ),
                        };
                        let first = out.len();
                        collect(&item.blocks, indent + 1, out);
                        if let Some(line) = out.get_mut(first) {
                            *line = format!("{}{marker} {line}", "  ".repeat(indent));
                        } else {
                            out.push(format!("{}{marker}", "  ".repeat(indent)));
                        }
                    }
                }
                MdBlock::Table(t) => {
                    out.push(
                        t.header
                            .iter()
                            .map(|c| inline(c))
                            .collect::<Vec<_>>()
                            .join("\t"),
                    );
                    for row in &t.rows {
                        out.push(row.iter().map(|c| inline(c)).collect::<Vec<_>>().join("\t"));
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    collect(blocks, 0, &mut out);
    out.join("\n")
}

pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<Preview, FmError> {
    let (bytes, more) = text::read_capped(vfs, entry, limits.text_bytes.min(MAX_BYTES))?;
    let total = entry.size.unwrap_or(bytes.len() as u64);
    let mut doc = parse_document(&text::decode(&bytes), limits.text_lines, total);
    doc.truncated |= more;
    Ok(Preview::Markdown(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_breaks_and_inert_resources() {
        let b = parse(
            "# Title\n\n**bold *both*** ~~gone~~ `c` [link](https://example.org)\nsoft  \nhard\n\n![alt](https://example.org/a.png)\n\n<script>no()</script>",
        );
        assert!(matches!(b.first(), Some(MdBlock::Heading(1, _))));
        let MdBlock::Para(p) = &b[1] else {
            panic!("paragraph")
        };
        assert!(p.iter().any(|i| i.bold && i.italic && i.text == "both"));
        assert!(p.iter().any(|i| i.strike && i.text == "gone"));
        assert!(p.iter().any(|i| i.code && i.text == "c"));
        assert!(
            p.iter()
                .any(|i| i.link.as_deref() == Some("https://example.org"))
        );
        assert!(plain_text(&b).contains(" soft\nhard"));
        assert!(plain_text(&b).contains("[Image: alt]"));
        assert!(plain_text(&b).contains("<script>no()</script>"));
    }

    #[test]
    fn nested_and_loose_lists_keep_every_paragraph_and_task() {
        let b = parse(
            "3. first\n\n   second paragraph\n\n   - [x] nested\n   - [ ] pending\n\n4. last\n\noutside",
        );
        let MdBlock::List { start, items } = &b[0] else {
            panic!("list")
        };
        assert_eq!(*start, Some(3));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].blocks.len(), 3);
        let MdBlock::List { items: nested, .. } = &items[0].blocks[2] else {
            panic!("nested list")
        };
        assert_eq!(nested[0].checked, Some(true));
        assert_eq!(nested[1].checked, Some(false));
        assert_eq!(
            plain_text(&b),
            "3. first\nsecond paragraph\n  [x] nested\n  [ ] pending\n4. last\noutside"
        );
    }

    #[test]
    fn quotes_tables_and_code_retain_structure() {
        let b = parse(
            "> first\n>\n> > nested\n>\n> last\n\n| A | B | C |\n| :-- | :-: | --: |\n| **one** | `two` | three |\n\n```rust title\nfn main() { let n = 3; }\n```\n",
        );
        let MdBlock::Quote(children) = &b[0] else {
            panic!("quote")
        };
        assert!(matches!(&children[1], MdBlock::Quote(_)));
        let MdBlock::Table(table) = &b[1] else {
            panic!("table")
        };
        assert_eq!(
            table.alignments,
            [Alignment::Left, Alignment::Center, Alignment::Right]
        );
        assert_eq!(table.header.len(), 3);
        assert!(table.rows[0][0][0].bold);
        let MdBlock::Code(code) = &b[2] else {
            panic!("code")
        };
        assert_eq!(code.language, "rust");
        assert!(
            code.lines
                .iter()
                .flatten()
                .any(|s| s.class == crate::Class::Keyword)
        );
        assert!(plain_text(&b).contains("A\tB\tC\none\ttwo\tthree"));
        assert!(plain_text(&b).contains("fn main() { let n = 3; }"));
    }

    #[test]
    fn unknown_code_language_and_long_lines_keep_the_original_text() {
        let line = format!("  {}  ", "x".repeat(5_000));
        let doc = parse(&format!("```not-a-language\n{line}\n```\n"));
        let MdBlock::Code(code) = &doc[0] else {
            panic!("code")
        };
        assert_eq!(code.language, "not-a-language");
        assert_eq!(code.text, line);
        assert_eq!(plain_text(&doc), line);
        assert_eq!(
            plain_text(&parse("first\r\nsecond  \r\nthird")),
            "first second\nthird"
        );
    }

    #[test]
    fn render_enforces_read_and_line_limits() {
        use fm_testkit::MemFs;
        use fm_vfs::VfsUri;
        let fs = MemFs::new();
        fs.put_str("/readme.md", b"# Title\n\nparagraph\n\nlast")
            .expect("file");
        let entry = fs
            .stat(&VfsUri::parse("/readme.md").expect("uri"))
            .expect("stat");
        for limits in [
            Limits {
                text_bytes: 8,
                ..Default::default()
            },
            Limits {
                text_lines: 3,
                ..Default::default()
            },
        ] {
            let Preview::Markdown(doc) = render(&fs, &entry, &limits).expect("render") else {
                panic!("markdown")
            };
            assert!(doc.truncated);
            assert!(!plain_text(&doc.blocks).contains("last"));
            assert_eq!(doc.total_bytes, entry.size.expect("size"));
        }
    }

    #[test]
    fn hostile_input_is_bounded_and_reports_truncation() {
        for source in [
            "- x\n".repeat(50_000),
            "> ".repeat(5_000),
            format!(
                "{}\n{}\n{}",
                "|x".repeat(100),
                "|-".repeat(100),
                "|y".repeat(100)
            ),
            "**x** ".repeat(50_000),
            "漢".repeat(MAX_BYTES),
        ] {
            let doc = parse_document(&source, 20_000, source.len() as u64);
            assert!(doc.truncated);
        }
        let doc = parse_document("a\nb\nc", 2, 5);
        assert!(doc.truncated);
        assert!(!plain_text(&doc.blocks).contains('c'));
        let doc = parse_document("a", 20_000, 1);
        assert!(!doc.truncated);
    }
}
