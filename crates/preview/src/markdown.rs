//! Markdown rendered to a flat block model. Links are shown, never followed; raw HTML
//! and images are shown as their text.

use fm_vfs::{Entry, FmError, Vfs};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::{Limits, Preview, text};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MdBlock {
    Heading(u8, Vec<Inline>),
    Para(Vec<Inline>),
    Code(String),
    Item(usize, String, Vec<Inline>),
    Quote(Vec<Inline>),
    Rule,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MdDoc {
    pub blocks: Vec<MdBlock>,
    pub truncated: bool,
    pub total_bytes: u64,
}

const MAX_BLOCKS: usize = 5000;

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

pub fn parse(src: &str) -> Vec<MdBlock> {
    let mut blocks = Vec::new();
    let mut cur: Vec<Inline> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut link: Option<String> = None;
    let mut heading: Option<u8> = None;
    let mut quote = 0u32;
    let mut code: Option<String> = None;
    // (ordered counter, depth) per open list
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut item: Option<(usize, String)> = None;

    let flush = |cur: &mut Vec<Inline>,
                 blocks: &mut Vec<MdBlock>,
                 heading: &mut Option<u8>,
                 item: &mut Option<(usize, String)>,
                 quote: u32| {
        if cur.is_empty() {
            return;
        }
        let inl = std::mem::take(cur);
        blocks.push(if let Some(h) = heading.take() {
            MdBlock::Heading(h, inl)
        } else if let Some((d, m)) = item.take() {
            MdBlock::Item(d, m, inl)
        } else if quote > 0 {
            MdBlock::Quote(inl)
        } else {
            MdBlock::Para(inl)
        });
    };

    for ev in Parser::new_ext(src, Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES) {
        if blocks.len() >= MAX_BLOCKS {
            break;
        }
        match ev {
            Event::Start(Tag::Heading { level: l, .. }) => {
                flush(&mut cur, &mut blocks, &mut heading, &mut item, quote);
                heading = Some(level(l));
            }
            Event::Start(Tag::BlockQuote(_)) => quote += 1,
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut cur, &mut blocks, &mut heading, &mut item, quote);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    blocks.push(MdBlock::Code(c.trim_end_matches('\n').to_string()));
                }
            }
            Event::Start(Tag::List(start)) => lists.push(start),
            Event::End(TagEnd::List(_)) => {
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut cur, &mut blocks, &mut heading, &mut item, quote);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}.");
                        *n += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                item = Some((lists.len().saturating_sub(1), marker));
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            Event::End(TagEnd::Link) => link = None,
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::Item
                | TagEnd::TableRow
                | TagEnd::TableHead,
            ) => flush(&mut cur, &mut blocks, &mut heading, &mut item, quote),
            Event::End(TagEnd::TableCell) => cur.push(Inline {
                text: "  │  ".into(),
                ..Inline::default()
            }),
            Event::Rule => {
                flush(&mut cur, &mut blocks, &mut heading, &mut item, quote);
                blocks.push(MdBlock::Rule);
            }
            Event::SoftBreak | Event::HardBreak => cur.push(Inline {
                text: " ".into(),
                bold: bold > 0,
                italic: italic > 0,
                ..Inline::default()
            }),
            Event::Text(t) | Event::Html(t) | Event::InlineHtml(t) => {
                if let Some(c) = code.as_mut() {
                    c.push_str(&t);
                } else {
                    cur.push(Inline {
                        text: t.to_string(),
                        bold: bold > 0,
                        italic: italic > 0,
                        code: false,
                        link: link.clone(),
                    });
                }
            }
            Event::Code(t) => cur.push(Inline {
                text: t.to_string(),
                code: true,
                link: link.clone(),
                ..Inline::default()
            }),
            _ => {}
        }
    }
    flush(&mut cur, &mut blocks, &mut heading, &mut item, quote);
    blocks
}

pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<Preview, FmError> {
    let (bytes, more) = text::read_capped(vfs, entry, limits.text_bytes)?;
    let total = entry.size.unwrap_or(bytes.len() as u64);
    let blocks = parse(&text::decode(&bytes));
    Ok(Preview::Markdown(MdDoc {
        truncated: more || blocks.len() >= MAX_BLOCKS,
        blocks,
        total_bytes: total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_and_inlines() {
        let b = parse(
            "# Title\n\nHello **bold** and `c` [l](http://x)\n\n- a\n- b\n\n1. one\n\n```\ncode\n```\n---\n",
        );
        assert!(matches!(&b[0], MdBlock::Heading(1, i) if i[0].text == "Title"));
        let MdBlock::Para(p) = &b[1] else {
            panic!("para")
        };
        assert!(p.iter().any(|i| i.bold && i.text == "bold"));
        assert!(p.iter().any(|i| i.code));
        assert!(p.iter().any(|i| i.link.as_deref() == Some("http://x")));
        assert!(
            b.iter()
                .any(|x| matches!(x, MdBlock::Item(0, m, _) if m == "•"))
        );
        assert!(
            b.iter()
                .any(|x| matches!(x, MdBlock::Item(0, m, _) if m == "1."))
        );
        assert!(
            b.iter()
                .any(|x| matches!(x, MdBlock::Code(c) if c == "code"))
        );
        assert!(b.contains(&MdBlock::Rule));
    }

    #[test]
    fn hostile_input_is_bounded() {
        let b = parse(&"- x\n".repeat(50_000));
        assert!(b.len() <= MAX_BLOCKS + 1);
        let _ = parse(&"> ".repeat(5000));
    }
}
