//! Text extraction for Find, selection and copy: the glyphs a page draws, with their
//! Unicode text and position, put into reading order line by line.

use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::{
    BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
    InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
};
use hayro::kurbo::{Affine, BezPath, Point, Rect};

use crate::doc::Document;
use crate::{PdfError, guard};

/// Characters kept per page.
pub const MAX_PAGE_CHARS: usize = 200_000;

/// One character of extracted text and where it is drawn (page space).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextChar {
    /// Byte offset of the character in [`PageText::text`].
    pub byte: u32,
    /// `[x0, y0, x1, y1]` in page space.
    pub rect: [f32; 4],
}

/// The text of one page.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    /// Lines separated by `\n`, words by spaces (inserted where glyphs leave a gap).
    pub text: String,
    /// One entry per character of `text`, in order.
    pub chars: Vec<TextChar>,
    /// Glyphs were found but some carried no Unicode mapping.
    pub unmapped: usize,
    /// The page has more characters than [`MAX_PAGE_CHARS`].
    pub truncated: bool,
}

impl PageText {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Rectangles covering byte range `start..end` of the text, merged per line.
    pub fn rects(&self, start: usize, end: usize) -> Vec<[f32; 4]> {
        let mut out: Vec<[f32; 4]> = Vec::new();
        for c in &self.chars {
            let b = c.byte as usize;
            if b < start || b >= end || c.rect[2] <= c.rect[0] {
                continue;
            }
            match out.last_mut() {
                Some(last)
                    if (last[1] - c.rect[1]).abs() < 0.5 * (last[3] - last[1]).max(1.0)
                        && c.rect[0] >= last[0] - 1.0 =>
                {
                    last[0] = last[0].min(c.rect[0]);
                    last[1] = last[1].min(c.rect[1]);
                    last[2] = last[2].max(c.rect[2]);
                    last[3] = last[3].max(c.rect[3]);
                }
                _ => out.push(c.rect),
            }
        }
        out
    }

    /// Byte offset of the character nearest to `(x, y)`; the offset after it when the
    /// point is in its right half.
    pub fn offset_at(&self, x: f32, y: f32) -> Option<usize> {
        let mut best: Option<(f32, usize)> = None;
        for (i, c) in self.chars.iter().enumerate() {
            let r = c.rect;
            if r[2] <= r[0] {
                continue;
            }
            let dx = if x < r[0] {
                r[0] - x
            } else if x > r[2] {
                x - r[2]
            } else {
                0.0
            };
            let dy = if y < r[1] {
                r[1] - y
            } else if y > r[3] {
                y - r[3]
            } else {
                0.0
            };
            // Vertical distance counts more: stay on the line the pointer is on.
            let d = dx + dy * 4.0;
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        let (_, i) = best?;
        let c = self.chars.get(i)?;
        let next = self
            .chars
            .get(i + 1)
            .map_or(self.text.len(), |n| n.byte as usize);
        Some(if x > (c.rect[0] + c.rect[2]) / 2.0 {
            next
        } else {
            c.byte as usize
        })
    }
}

#[derive(Clone, Debug)]
struct Glyph {
    text: String,
    rect: Rect,
    /// Baseline origin and the em height in page space.
    origin: Point,
    size: f64,
    advance: f64,
}

struct Collector {
    glyphs: Vec<Glyph>,
    unmapped: usize,
    full: bool,
}

impl<'a> Device<'a> for Collector {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for g in run.glyphs() {
            if self.glyphs.len() >= MAX_PAGE_CHARS {
                self.full = true;
                return;
            }
            let t: Affine = props.transform * g.transform();
            let text = match g.as_unicode() {
                Some(BfString::Char(c)) => c.to_string(),
                Some(BfString::String(s)) => s.chars().take(8).collect(),
                None => {
                    self.unmapped += 1;
                    continue;
                }
            };
            if text.chars().all(char::is_control) {
                continue;
            }
            let advance = match g.glyph_advance() {
                Some(a) if a.is_finite() && a > 0.0 => a,
                _ => 500.0,
            };
            let corners = [
                t * Point::new(0.0, -200.0),
                t * Point::new(advance, -200.0),
                t * Point::new(0.0, 800.0),
                t * Point::new(advance, 800.0),
            ];
            let mut rect = Rect::from_points(corners[0], corners[1]);
            for c in &corners[2..] {
                rect = rect.union_pt(*c);
            }
            let origin = t * Point::ZERO;
            let up = t * Point::new(0.0, 1000.0);
            let size = ((up.x - origin.x).powi(2) + (up.y - origin.y).powi(2)).sqrt();
            let end = t * Point::new(advance, 0.0);
            let adv = ((end.x - origin.x).powi(2) + (end.y - origin.y).powi(2)).sqrt();
            if !rect.is_finite() || !size.is_finite() {
                continue;
            }
            self.glyphs.push(Glyph {
                text,
                rect,
                origin,
                size,
                advance: adv,
            });
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

trait GlyphAdvance {
    fn glyph_advance(&self) -> Option<f64>;
}

impl GlyphAdvance for hayro::hayro_interpret::font::PositionedGlyph<'_> {
    fn glyph_advance(&self) -> Option<f64> {
        match &**self {
            hayro::hayro_interpret::font::Glyph::Outline(o) => o.advance_width().map(f64::from),
            hayro::hayro_interpret::font::Glyph::Type3(t) => t.advance_width().map(f64::from),
        }
    }
}

/// Extract the text of page `index`.
pub fn page_text(doc: &Document, index: usize) -> Result<PageText, PdfError> {
    guard(|| {
        let page = doc.page(index)?;
        let transform = page.initial_transform(true).to_kurbo();
        let cache = InterpreterCache::new();
        let info = doc
            .info()
            .pages
            .get(index)
            .ok_or_else(|| PdfError::Refused(format!("page {} does not exist", index + 1)))?;
        let bbox = Rect::new(
            -1.0,
            -1.0,
            f64::from(info.width) + 1.0,
            f64::from(info.height) + 1.0,
        );
        let mut ctx = Context::new(
            transform,
            bbox,
            &cache,
            doc.pdf().xref(),
            InterpreterSettings::default(),
        );
        let mut c = Collector {
            glyphs: Vec::new(),
            unmapped: 0,
            full: false,
        };
        interpret_page(page, &mut ctx, &mut c);
        Ok(assemble(c))
    })
}

/// Put glyphs into lines and words. Content order is kept within a line (it is reading
/// order in nearly every producer); a new line starts when the baseline moves.
fn assemble(c: Collector) -> PageText {
    let mut out = PageText {
        unmapped: c.unmapped,
        truncated: c.full,
        ..Default::default()
    };
    let mut prev: Option<&Glyph> = None;
    for g in &c.glyphs {
        if let Some(p) = prev {
            let line_h = p.size.max(g.size).max(1.0);
            let dy = (g.origin.y - p.origin.y).abs();
            let gap = g.origin.x - (p.origin.x + p.advance);
            let new_line = dy > line_h * 0.5 || gap < -line_h * 2.0;
            let pad = |out: &mut PageText, ch: char, at: Rect| {
                out.chars.push(TextChar {
                    byte: out.text.len() as u32,
                    rect: [at.x0 as f32, at.y0 as f32, at.x0 as f32, at.y1 as f32],
                });
                out.text.push(ch);
            };
            if new_line {
                if !out.text.ends_with('\n') {
                    pad(&mut out, '\n', p.rect);
                }
            } else if gap > line_h * 0.2 && !out.text.ends_with(' ') && !g.text.starts_with(' ') {
                let at = Rect::new(p.rect.x1, p.rect.y0, g.rect.x0.max(p.rect.x1), p.rect.y1);
                out.chars.push(TextChar {
                    byte: out.text.len() as u32,
                    rect: [at.x0 as f32, at.y0 as f32, at.x1 as f32, at.y1 as f32],
                });
                out.text.push(' ');
            }
        }
        let n = g.text.chars().count().max(1) as f64;
        let w = g.rect.width() / n;
        for (i, ch) in g.text.chars().enumerate() {
            let x0 = g.rect.x0 + w * i as f64;
            out.chars.push(TextChar {
                byte: out.text.len() as u32,
                rect: [
                    x0 as f32,
                    g.rect.y0 as f32,
                    (x0 + w) as f32,
                    g.rect.y1 as f32,
                ],
            });
            out.text.push(ch);
        }
        prev = Some(g);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn extracts_text_in_order_with_positions() {
        let doc = Document::open(fixtures::text_pages(3).into(), None).expect("open");
        let t = page_text(&doc, 1).expect("text");
        assert!(t.text.contains("Page 2"), "{:?}", t.text);
        assert!(t.text.contains("invoice"), "{:?}", t.text);
        assert_eq!(t.chars.len(), t.text.chars().count());
        let at = t.text.find("invoice").expect("word");
        let rects = t.rects(at, at + "invoice".len());
        assert_eq!(rects.len(), 1);
        let r = rects[0];
        assert!(r[0] < r[2] && r[1] < r[3]);
        assert!(r[1] > 0.0 && r[3] < 842.0, "inside the page: {r:?}");
        // Lines come out on separate lines.
        assert!(t.text.lines().count() >= 2, "{:?}", t.text);
        let off = t
            .offset_at(r[0] + 0.1, (r[1] + r[3]) / 2.0)
            .expect("offset");
        assert_eq!(off, at);
    }

    #[test]
    fn scanned_page_has_no_text() {
        let doc = Document::open(fixtures::image_only().into(), None).expect("open");
        assert!(page_text(&doc, 0).expect("text").is_empty());
    }
}
