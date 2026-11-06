//! A tolerant reader for the small HTML subset README files use. It never executes or loads
//! anything: tags are matched against an allow-list, attributes are copied only for a few
//! names and length-bounded, `script`/`style` content is dropped, and everything unknown is
//! transparent (its text stays). Output is the ordinary Markdown model.

use super::{
    Alignment, Budget, Inline, MAX_ALT_CHARS, MAX_CELLS, MAX_COLUMNS, MAX_DEPTH, MAX_IMAGES,
    MAX_INLINES, MdBlock, MdImage, Table,
};

pub(super) const MAX_INPUT: usize = 256 * 1024;
const MAX_TOKENS: usize = 20_000;
const MAX_TAG: usize = 4_096;
const MAX_ATTR: usize = 2_048;
const MAX_ATTRS: usize = 16;
const MAX_NAME: usize = 16;
const MAX_WIDTH: u32 = 4_096;

pub(super) struct Tag {
    pub name: String,
    pub closing: bool,
    attrs: Vec<(String, String)>,
}

impl Tag {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

pub(super) enum Token {
    Text(String),
    Tag(Tag),
}

fn named_entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        "middot" => '·',
        "copy" => '©',
        "ndash" => '–',
        "mdash" => '—',
        "hellip" => '…',
        "bull" => '•',
        _ => return None,
    })
}

fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let tail = rest.get(at + 1..).unwrap_or("");
        let decoded = tail
            .char_indices()
            .take(10)
            .find(|&(_, c)| c == ';')
            .and_then(|(end, _)| {
                let body = tail.get(..end)?;
                let c = match body.strip_prefix('#') {
                    Some(num) => {
                        let code = match num.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                            None => num.parse().ok()?,
                        };
                        char::from_u32(code).filter(|c| !c.is_control())?
                    }
                    None => named_entity(body)?,
                };
                Some((c, end + 1))
            });
        match decoded {
            Some((c, used)) => {
                out.push(c);
                rest = tail.get(used..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The tag starting at the beginning of `s` (which starts with `<`) and its byte length.
fn read_tag(s: &str) -> Option<(Tag, usize)> {
    let mut quote = None;
    let mut end = None;
    for (i, c) in s.char_indices() {
        if i > MAX_TAG {
            return None;
        }
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                end = Some(i);
                break;
            }
            _ => {}
        }
    }
    let end = end?;
    let inner = s.get(1..end)?;
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    if !inner.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_end = inner
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(inner.len());
    let name = inner
        .get(..name_end)?
        .chars()
        .take(MAX_NAME)
        .collect::<String>()
        .to_ascii_lowercase();
    let mut rest = inner.get(name_end..)?;
    let mut attrs = Vec::new();
    while attrs.len() < MAX_ATTRS {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() {
            break;
        }
        let key_end = rest
            .find(|c: char| c.is_whitespace() || c == '=' || c == '/')
            .unwrap_or(rest.len())
            .max(rest.chars().next().map_or(1, char::len_utf8));
        let key = rest.get(..key_end)?.to_ascii_lowercase();
        rest = rest.get(key_end..)?.trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            let (raw, tail) = match after.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = after.get(1..)?;
                    let close = body.find(q).unwrap_or(body.len());
                    (body.get(..close)?, body.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let close = after.find(char::is_whitespace).unwrap_or(after.len());
                    (after.get(..close)?, after.get(close..)?)
                }
            };
            value = decode(raw).chars().take(MAX_ATTR).collect();
            rest = tail;
        }
        if matches!(
            key.as_str(),
            "src" | "srcset" | "alt" | "width" | "href" | "align" | "media" | "title"
        ) {
            attrs.push((key, value));
        }
    }
    Some((
        Tag {
            name,
            closing,
            attrs,
        },
        end + 1,
    ))
}

/// Byte offset just past the next `</name ...>` (ASCII case-insensitive), or the end.
fn skip_raw_text(s: &str, name: &str) -> usize {
    let needle = format!("</{name}").into_bytes();
    let found = s
        .as_bytes()
        .windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(&needle));
    match found {
        Some(at) => s
            .get(at..)
            .and_then(|t| t.find('>'))
            .map_or(s.len(), |gt| at + gt + 1),
        None => s.len(),
    }
}

pub(super) fn tokenize(src: &str) -> Vec<Token> {
    let mut end = src.len().min(MAX_INPUT);
    while !src.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut rest = src.get(..end).unwrap_or("");
    let mut out = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut Vec<Token>| {
        if !text.is_empty() {
            out.push(Token::Text(decode(text)));
            text.clear();
        }
    };
    while !rest.is_empty() && out.len() < MAX_TOKENS {
        let Some(lt) = rest.find('<') else {
            text.push_str(rest);
            break;
        };
        text.push_str(rest.get(..lt).unwrap_or(""));
        let after = rest.get(lt..).unwrap_or("");
        if let Some(body) = after.strip_prefix("<!--") {
            rest = body
                .find("-->")
                .and_then(|e| body.get(e + 3..))
                .unwrap_or("");
            continue;
        }
        match read_tag(after) {
            Some((tag, used)) => {
                flush(&mut text, &mut out);
                rest = after.get(used..).unwrap_or("");
                if !tag.closing && matches!(tag.name.as_str(), "script" | "style") {
                    rest = rest.get(skip_raw_text(rest, &tag.name)..).unwrap_or("");
                } else {
                    out.push(Token::Tag(tag));
                }
            }
            None => {
                text.push('<');
                rest = after.get(1..).unwrap_or("");
            }
        }
    }
    flush(&mut text, &mut out);
    out
}

pub(super) fn alt_of(tag: &Tag) -> String {
    tag.attr("alt")
        .unwrap_or("")
        .chars()
        .take(MAX_ALT_CHARS)
        .collect()
}

fn width_of(tag: &Tag) -> Option<u32> {
    let value = tag.attr("width")?.trim();
    let digits = value.strip_suffix("px").unwrap_or(value);
    digits
        .parse::<u32>()
        .ok()
        .filter(|w| (1..=MAX_WIDTH).contains(w))
}

/// The first URL of a `srcset`.
fn first_source(srcset: &str) -> Option<String> {
    let first = srcset.split(',').next()?.split_whitespace().next()?;
    Some(first.to_string())
}

fn is_dark_media(media: &str) -> bool {
    let squashed: String = media.chars().filter(|c| !c.is_whitespace()).collect();
    squashed
        .to_ascii_lowercase()
        .contains("prefers-color-scheme:dark")
}

/// An `<img>` as the model's image; `dark` comes from an enclosing `<picture>`.
pub(super) fn image(tag: &Tag, dark: Option<String>, link: Option<String>) -> Option<MdImage> {
    let url = tag.attr("src").filter(|s| !s.trim().is_empty())?;
    Some(MdImage {
        alt: alt_of(tag),
        url: url.trim().to_string(),
        title: tag.attr("title").unwrap_or("").to_string(),
        link,
        dark_url: dark,
        width: width_of(tag),
    })
}

struct Frame {
    name: String,
    center: bool,
}

struct Cell {
    inlines: Vec<Inline>,
    images: Vec<MdImage>,
    align: Alignment,
    header: bool,
}

#[derive(Default)]
struct TableBuilder {
    rows: Vec<Vec<Cell>>,
    open: Option<Cell>,
    row_open: bool,
}

struct Builder<'a> {
    budget: &'a mut Budget,
    frames: Vec<Frame>,
    out: Vec<MdBlock>,
    inlines: Vec<Inline>,
    images: Vec<MdImage>,
    bold: usize,
    italic: usize,
    code: usize,
    link: Option<String>,
    heading: Option<u8>,
    picture: Option<Option<String>>,
    table: Option<TableBuilder>,
    nested_tables: usize,
}

fn heading_level(name: &str) -> Option<u8> {
    let digit = name.strip_prefix('h')?;
    digit.parse().ok().filter(|n| (1..=6).contains(n))
}

fn step(counter: &mut usize, closing: bool) {
    if closing {
        *counter = counter.saturating_sub(1);
    } else {
        *counter += 1;
    }
}

impl Builder<'_> {
    fn at_break(&self) -> bool {
        self.inlines
            .last()
            .is_none_or(|i| i.text.ends_with([' ', '\n']))
    }

    fn push_inline(&mut self, text: String) {
        if self.budget.inlines >= MAX_INLINES {
            self.budget.stop();
            return;
        }
        self.budget.inlines += 1;
        self.inlines.push(Inline {
            text,
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: false,
            code: self.code > 0,
            link: self.link.clone(),
        });
    }

    fn text(&mut self, text: &str) {
        let mut space = self.at_break();
        let mut collapsed = String::new();
        for c in text.chars() {
            if c.is_whitespace() {
                if !space {
                    collapsed.push(' ');
                    space = true;
                }
            } else {
                collapsed.push(c);
                space = false;
            }
        }
        if !collapsed.is_empty() {
            self.push_inline(collapsed);
        }
    }

    fn trim(&mut self) {
        if let Some(last) = self.inlines.last_mut() {
            let kept = last.text.trim_end().len();
            last.text.truncate(kept);
        }
        self.inlines.retain(|i| !i.text.is_empty());
    }

    fn emit(&mut self, block: MdBlock) {
        if !self.budget.block() {
            return;
        }
        self.out.push(if self.frames.iter().any(|f| f.center) {
            MdBlock::Centered(vec![block])
        } else {
            block
        });
    }

    /// Ends the current paragraph; inside a table cell content keeps accumulating there.
    fn flush(&mut self) {
        if self.table.is_some() {
            return;
        }
        self.trim();
        let inlines = std::mem::take(&mut self.inlines);
        let images = std::mem::take(&mut self.images);
        if !inlines.is_empty() {
            self.emit(match self.heading {
                Some(level) => MdBlock::Heading(level, inlines),
                None => MdBlock::Para(inlines),
            });
        }
        if !images.is_empty() {
            self.emit(MdBlock::Images(images));
        }
    }

    fn close_cell(&mut self) {
        if self.table.is_none() {
            return;
        }
        self.trim();
        let inlines = std::mem::take(&mut self.inlines);
        let images = std::mem::take(&mut self.images);
        let Some(table) = self.table.as_mut() else {
            return;
        };
        if let Some(mut cell) = table.open.take() {
            cell.inlines = inlines;
            cell.images = images;
            if let Some(row) = table.rows.last_mut() {
                row.push(cell);
            }
        }
    }

    fn close_row(&mut self) {
        self.close_cell();
        if let Some(table) = self.table.as_mut() {
            table.row_open = false;
        }
    }

    fn open_cell(&mut self, tag: &Tag) {
        self.close_cell();
        self.inlines.clear();
        self.images.clear();
        let cells = self.budget.cells;
        let Some(table) = self.table.as_mut() else {
            return;
        };
        if !table.row_open {
            table.rows.push(Vec::new());
            table.row_open = true;
        }
        let columns = table.rows.last().map_or(0, Vec::len);
        if cells >= MAX_CELLS || columns >= MAX_COLUMNS {
            self.budget.stop();
            return;
        }
        self.budget.cells += 1;
        table.open = Some(Cell {
            inlines: Vec::new(),
            images: Vec::new(),
            align: match tag.attr("align").map(str::to_ascii_lowercase).as_deref() {
                Some("center") => Alignment::Center,
                Some("right") => Alignment::Right,
                _ => Alignment::Left,
            },
            header: tag.name == "th",
        });
    }

    fn finish_table(&mut self) {
        self.close_row();
        let Some(builder) = self.table.take() else {
            return;
        };
        self.nested_tables = 0;
        let columns = builder.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let headerless = !builder
            .rows
            .first()
            .is_some_and(|row| row.iter().any(|c| c.header));
        let alignments = (0..columns)
            .map(|col| {
                builder
                    .rows
                    .iter()
                    .filter_map(|row| row.get(col))
                    .map(|c| c.align)
                    .find(|a| *a != Alignment::Left)
                    .unwrap_or(Alignment::Left)
            })
            .collect();
        let mut table = Table {
            alignments,
            header: Vec::new(),
            rows: Vec::new(),
            images: Vec::new(),
            headerless,
        };
        if headerless {
            table.images.push(Vec::new());
        }
        for (n, row) in builder.rows.into_iter().enumerate() {
            let (cells, pictures): (Vec<_>, Vec<_>) =
                row.into_iter().map(|c| (c.inlines, c.images)).unzip();
            table.images.push(pictures);
            if n == 0 && !headerless {
                table.header = cells;
            } else {
                table.rows.push(cells);
            }
        }
        self.emit(MdBlock::Table(table));
    }

    fn container(&mut self, tag: &Tag) {
        self.flush();
        if tag.closing {
            if let Some(at) = self.frames.iter().rposition(|f| f.name == tag.name) {
                self.frames.truncate(at);
            }
            self.heading = None;
            return;
        }
        if self.frames.len() < MAX_DEPTH {
            self.frames.push(Frame {
                name: tag.name.clone(),
                center: tag.name == "center"
                    || tag
                        .attr("align")
                        .is_some_and(|a| a.eq_ignore_ascii_case("center")),
            });
        }
        self.heading = heading_level(&tag.name);
    }

    fn tag(&mut self, tag: Tag) {
        let name = tag.name.as_str();
        let closing = tag.closing;
        if matches!(name, "p" | "div" | "center") || heading_level(name).is_some() {
            if self.table.is_none() {
                self.container(&tag);
            }
            return;
        }
        match name {
            "br" => {
                if !self.inlines.is_empty() {
                    self.trim();
                    self.push_inline("\n".into());
                }
            }
            "b" | "strong" => step(&mut self.bold, closing),
            "i" | "em" => step(&mut self.italic, closing),
            "code" | "tt" | "kbd" => step(&mut self.code, closing),
            "a" => {
                self.link = if closing {
                    None
                } else {
                    tag.attr("href").filter(|h| !h.is_empty()).map(String::from)
                };
            }
            "img" if !closing => {
                let dark = self.picture.clone().flatten();
                if self.budget.images >= MAX_IMAGES {
                    self.budget.truncated = true;
                } else if let Some(img) = image(&tag, dark, self.link.clone()) {
                    self.budget.images += 1;
                    self.images.push(img);
                }
            }
            "picture" => self.picture = if closing { None } else { Some(None) },
            "source" if !closing => {
                if let Some(dark) = self.picture.as_mut().filter(|d| d.is_none())
                    && tag.attr("media").is_some_and(is_dark_media)
                {
                    *dark = tag.attr("srcset").and_then(first_source);
                }
            }
            "table" if !closing && self.table.is_some() => self.nested_tables += 1,
            "table" if closing && self.nested_tables > 0 => self.nested_tables -= 1,
            "table" if !closing => {
                self.flush();
                self.table = Some(TableBuilder::default());
            }
            "table" => self.finish_table(),
            _ if self.nested_tables > 0 => {}
            "tr" if self.table.is_some() => {
                self.close_row();
                self.inlines.clear();
                self.images.clear();
            }
            "td" | "th" if closing => self.close_cell(),
            "td" | "th" if self.table.is_some() => self.open_cell(&tag),
            "hr" if !closing && self.table.is_none() => {
                self.flush();
                self.emit(MdBlock::Rule);
            }
            "ul" | "ol" | "li" | "pre" | "blockquote" => self.flush(),
            _ => {}
        }
    }
}

/// Blocks for one HTML fragment; an unclosed element simply ends with the fragment.
pub(super) fn blocks(src: &str, budget: &mut Budget) -> Vec<MdBlock> {
    let mut b = Builder {
        budget,
        frames: Vec::new(),
        out: Vec::new(),
        inlines: Vec::new(),
        images: Vec::new(),
        bold: 0,
        italic: 0,
        code: 0,
        link: None,
        heading: None,
        picture: None,
        table: None,
        nested_tables: 0,
    };
    for token in tokenize(src) {
        if b.budget.exhausted {
            break;
        }
        match token {
            Token::Text(t) => b.text(&t),
            Token::Tag(tag) => b.tag(tag),
        }
    }
    b.finish_table();
    b.flush();
    b.out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vec<MdBlock> {
        blocks(src, &mut Budget::default())
    }

    fn text(blocks: &[MdBlock]) -> String {
        super::super::plain_text(blocks)
    }

    #[test]
    fn centered_heading_and_paragraph() {
        let out = run(
            "<h1 align=\"center\">Name &amp; Co</h1>\n<p align=center>a<br>b <code>c</code></p>",
        );
        assert_eq!(out.len(), 2);
        let MdBlock::Centered(h) = &out[0] else {
            panic!("{out:?}")
        };
        assert!(matches!(&h[0], MdBlock::Heading(1, p) if p[0].text == "Name & Co"));
        assert_eq!(text(&out[1..]), "a\nb c");
    }

    #[test]
    fn picture_keeps_dark_source_and_fallback() {
        let out = run(
            "<picture>\n<source media=\"(prefers-color-scheme: dark)\" srcset=\"d.webp\">\n<img src=\"l.webp\" alt=\"x\" width=\"96\">\n</picture>",
        );
        let [MdBlock::Images(images)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(images[0].url, "l.webp");
        assert_eq!(images[0].dark_url.as_deref(), Some("d.webp"));
        assert_eq!(images[0].width, Some(96));
    }

    #[test]
    fn table_of_pictures_is_headerless_grid() {
        let out = run(
            "<table><tr><td><img src=\"a.png\"></td><td><img src=\"b.png\"></td></tr><tr><td>t</td><td></td></tr></table>",
        );
        let [MdBlock::Table(t)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(t.headerless && t.header.is_empty());
        assert_eq!(t.rows.len(), 2);
        assert_eq!(t.images[1][1][0].url, "b.png");
        assert_eq!(t.rows[1][0][0].text, "t");
    }

    #[test]
    fn th_row_becomes_header() {
        let out = run("<table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>");
        let [MdBlock::Table(t)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert!(!t.headerless);
        assert_eq!(t.header.len(), 2);
        assert_eq!(t.rows.len(), 1);
    }

    #[test]
    fn links_keep_href_and_text() {
        let out = run("<p><a href=\"x.md\">One</a> · <a href=\"y\">Two</a></p>");
        let [MdBlock::Para(p)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(p[0].link.as_deref(), Some("x.md"));
        assert_eq!(p[1].text, " · ");
        assert_eq!(p[2].link.as_deref(), Some("y"));
    }

    #[test]
    fn scripts_styles_comments_and_unknown_tags() {
        let out = run(
            "<script>alert(1)</script><style>p{}</style><!-- hi --><blink>keep</blink><SCRIPT x='>'>no</SCRIPT>",
        );
        assert_eq!(text(&out), "keep");
    }

    #[test]
    fn hostile_input_stays_bounded() {
        let deep = "<div>".repeat(100_000);
        assert!(run(&deep).len() < 10);
        let open = "<a href=\"".repeat(10_000);
        let _ = run(&open);
        let big = format!("<p title=\"{}\">x</p>", "a".repeat(10_000_000));
        let _ = run(&big);
        let entities = "&#99999999999; &#x; &; &amp".repeat(50_000);
        let _ = run(&entities);
        let tags = "<td><tr><img src=a>".repeat(50_000);
        let mut budget = Budget::default();
        let _ = blocks(&tags, &mut budget);
        assert!(budget.images <= MAX_IMAGES);
        let _ = run("<");
        let _ = run("<p align='");
        let _ = run("é<é>é</é");
        let _ = run("<é é=é é>");
    }

    #[test]
    fn unknown_attributes_are_not_kept() {
        let tokens = tokenize("<img src=a onerror=\"x()\" style=\"y\">");
        let Some(Token::Tag(tag)) = tokens.first() else {
            panic!("no tag")
        };
        assert_eq!(tag.attr("src"), Some("a"));
        assert_eq!(tag.attr("onerror"), None);
        assert_eq!(tag.attrs.len(), 1);
    }

    #[test]
    fn widths_are_clamped() {
        let out =
            run("<img src=a width=\"99999\"><img src=b width=\"50%\"><img src=c width=\"40px\">");
        let [MdBlock::Images(images)] = out.as_slice() else {
            panic!("{out:?}")
        };
        assert_eq!(
            images.iter().map(|i| i.width).collect::<Vec<_>>(),
            [None, None, Some(40)]
        );
    }
}
