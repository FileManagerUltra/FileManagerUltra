//! Pages (or tiles of pages) to RGBA, with the page-colour modes for low-vision reading
//! applied to the pixels (design D4). The document itself is never changed.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::kurbo::Affine;
use hayro::vello_cpu::{self, RenderContext};
use hayro::{RenderCache, RenderSettings};

use crate::doc::Document;
use crate::{MAX_PIXELS, PdfError, guard};

/// How rendered pages are coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ColorMode {
    #[default]
    Original,
    /// Luminance inverted, hue kept: dark paper, light text, photos still recognizable.
    Dark,
    /// Page content mapped onto two theme colours (text on background).
    HighContrast { fg: [u8; 3], bg: [u8; 3] },
}

/// A part of the page bitmap at some scale, in pixels of the whole scaled page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Straight (opaque) RGBA pixels.
#[derive(Clone, PartialEq, Eq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({}x{})", self.width, self.height)
    }
}

impl Pixels {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }
}

/// Size in pixels of page `index` drawn at `scale` (pixels per point).
pub fn page_pixels(doc: &Document, index: usize, scale: f32) -> Option<(u32, u32)> {
    let p = doc.info().pages.get(index)?;
    let w = (p.width * scale).round().max(1.0);
    let h = (p.height * scale).round().max(1.0);
    if !w.is_finite() || !h.is_finite() || w > 1.0e7 || h > 1.0e7 {
        return None;
    }
    Some((w as u32, h as u32))
}

/// Render page `index` at `scale` pixels per point. Without a `tile` the whole page is
/// drawn and must fit [`MAX_PIXELS`]; with one, only that part (also capped).
pub fn render_page(
    doc: &Document,
    index: usize,
    scale: f32,
    colors: ColorMode,
    tile: Option<Tile>,
) -> Result<Pixels, PdfError> {
    if !(scale.is_finite() && scale > 0.0 && scale <= 64.0) {
        return Err(PdfError::Refused("invalid render scale".into()));
    }
    let (page_w, page_h) = page_pixels(doc, index, scale)
        .ok_or_else(|| PdfError::Refused(format!("page {} does not exist", index + 1)))?;
    let area = tile.unwrap_or(Tile {
        x: 0,
        y: 0,
        width: page_w,
        height: page_h,
    });
    let width = area.width.min(page_w.saturating_sub(area.x));
    let height = area.height.min(page_h.saturating_sub(area.y));
    if width == 0 || height == 0 {
        return Err(PdfError::Refused("the tile is outside the page".into()));
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS
        || width > u32::from(u16::MAX)
        || height > u32::from(u16::MAX)
    {
        return Err(PdfError::TooLarge(
            "the page is too large to draw at this zoom".into(),
        ));
    }
    let mut pixels = guard(|| {
        let page = doc.page(index)?;
        let cache = RenderCache::new();
        let settings = InterpreterSettings::default();
        let render_settings = RenderSettings::default();
        let mut ctx = RenderContext::new(width as u16, height as u16);
        let transform = Affine::translate((-(area.x as f64), -(area.y as f64)))
            * Affine::scale(scale as f64)
            * page.initial_transform(true).to_kurbo();
        hayro::render_into(
            page,
            &cache,
            &settings,
            &render_settings,
            &mut ctx,
            transform,
        );
        ctx.flush();
        let mut pixmap = vello_cpu::Pixmap::new(width as u16, height as u16);
        ctx.render_with(
            &mut pixmap,
            &mut vello_cpu::Resources::default(),
            vello_cpu::RasterizerSettings {
                target_init: vello_cpu::TargetInit::Clear(vello_cpu::color::palette::css::WHITE),
                ..Default::default()
            },
        );
        Ok(Pixels {
            width,
            height,
            rgba: pixmap.data_as_u8_slice().to_vec(),
        })
    })?;
    apply_colors(&mut pixels.rgba, colors);
    Ok(pixels)
}

/// Recolour opaque RGBA pixels in place.
pub fn apply_colors(rgba: &mut [u8], mode: ColorMode) {
    match mode {
        ColorMode::Original => {}
        ColorMode::Dark => {
            for px in rgba.as_chunks_mut::<4>().0 {
                let (r, g, b) = (f32::from(px[0]), f32::from(px[1]), f32::from(px[2]));
                let y = 0.299 * r + 0.587 * g + 0.114 * b;
                // White paper becomes a soft dark grey, black text a soft light grey.
                let ny = 28.0 + (232.0 - 28.0) * (1.0 - y / 255.0);
                let (cb, cr) = (b - y, r - y);
                let nr = ny + cr;
                let nb = ny + cb;
                let ng = (ny - 0.299 * nr - 0.114 * nb) / 0.587;
                px[0] = nr.clamp(0.0, 255.0) as u8;
                px[1] = ng.clamp(0.0, 255.0) as u8;
                px[2] = nb.clamp(0.0, 255.0) as u8;
            }
        }
        ColorMode::HighContrast { fg, bg } => {
            for px in rgba.as_chunks_mut::<4>().0 {
                let y =
                    0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2]);
                // Smooth step keeps anti-aliased edges instead of jagged thresholding.
                let t = ((y - 110.0) / (210.0 - 110.0)).clamp(0.0, 1.0);
                let t = t * t * (3.0 - 2.0 * t);
                for c in 0..3 {
                    let v = f32::from(fg[c]) + (f32::from(bg[c]) - f32::from(fg[c])) * t;
                    px[c] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn doc() -> Document {
        Document::open(fixtures::text_pages(3).into(), None).expect("open")
    }

    #[test]
    fn renders_a_page_with_text() {
        let d = doc();
        let px = render_page(&d, 0, 1.0, ColorMode::Original, None).expect("render");
        assert_eq!((px.width, px.height), (595, 842));
        assert_eq!(px.rgba.len(), 595 * 842 * 4);
        let dark = px
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] < 100)
            .count();
        assert!(dark > 100, "some text pixels are drawn ({dark})");
        assert!(
            px.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|p| p[0] == 255)
                .count()
                > 100_000
        );
    }

    #[test]
    fn tiles_and_caps() {
        let d = doc();
        let t = Tile {
            x: 100,
            y: 100,
            width: 64,
            height: 32,
        };
        let px = render_page(&d, 0, 2.0, ColorMode::Original, Some(t)).expect("tile");
        assert_eq!((px.width, px.height), (64, 32));
        // A4 at 10x is ~50 MP: refused whole, fine as a tile.
        assert!(matches!(
            render_page(&d, 0, 10.0, ColorMode::Original, None),
            Err(PdfError::TooLarge(_))
        ));
        assert!(
            render_page(
                &d,
                0,
                10.0,
                ColorMode::Original,
                Some(Tile {
                    x: 0,
                    y: 0,
                    width: 1024,
                    height: 1024
                })
            )
            .is_ok()
        );
        assert!(render_page(&d, 9, 1.0, ColorMode::Original, None).is_err());
        assert!(render_page(&d, 0, f32::NAN, ColorMode::Original, None).is_err());
        assert!(render_page(&d, 0, 0.0, ColorMode::Original, None).is_err());
    }

    #[test]
    fn rotation_is_honoured() {
        let d = Document::open(fixtures::rotated_page().into(), None).expect("open");
        let px = render_page(&d, 0, 0.5, ColorMode::Original, None).expect("render");
        assert!(px.width > px.height);
    }

    #[test]
    fn colour_modes() {
        let mut white = vec![255, 255, 255, 255, 0, 0, 0, 255, 200, 30, 30, 255];
        apply_colors(&mut white, ColorMode::Dark);
        assert!(white[0] < 60, "paper goes dark");
        assert!(white[4] > 200, "text goes light");
        assert!(white[8] > white[9], "red stays reddish");
        let mut px = vec![255, 255, 255, 255, 0, 0, 0, 255];
        apply_colors(
            &mut px,
            ColorMode::HighContrast {
                fg: [255, 255, 0],
                bg: [0, 0, 0],
            },
        );
        assert_eq!(&px[..3], &[0, 0, 0]);
        assert_eq!(&px[4..7], &[255, 255, 0]);
    }
}
