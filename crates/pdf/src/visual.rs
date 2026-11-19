//! Visual signatures (design D8): drawn strokes and typed names become vector paths (no
//! font is embedded in the PDF), images become an image XObject (JPEG passed through,
//! everything else Flate RGB with a soft mask).

use std::io::Write as _;
use std::sync::Arc;

use crate::lo::fmt_num;
use crate::{PdfError, guard};

/// Strokes per drawn signature.
pub const MAX_STROKES: usize = 64;
/// Points per stroke.
pub const MAX_POINTS: usize = 2048;
/// Encoded image size.
pub const MAX_IMAGE_BYTES: usize = 8 << 20;
/// Decoded image pixels.
pub const MAX_IMAGE_PIXELS: u64 = 4_000_000;
/// Characters of a typed signature.
pub const MAX_TYPED: usize = 128;

/// The bundled signature font (Dancing Script, SIL OFL 1.1).
pub const SIGNATURE_FONT: &[u8] = include_bytes!("../assets/DancingScript.ttf");
/// Its licence, for the notices shown in About.
pub const SIGNATURE_FONT_LICENSE: &str = include_str!("../assets/DancingScript-OFL.txt");

/// What the user made.
#[derive(Clone, Debug, PartialEq)]
pub enum VisualKind {
    /// Strokes in pad coordinates (x right, y down) on a `width` × `height` pad.
    Drawn {
        strokes: Vec<Vec<[f32; 2]>>,
        width: f32,
        height: f32,
    },
    /// A name set in the signature font.
    Typed { name: String },
    /// A PNG or JPEG; `remove_white` makes a near-white background transparent.
    Image { data: Vec<u8>, remove_white: bool },
}

/// An image XObject ready to write.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageObject {
    pub width: u32,
    pub height: u32,
    /// `DeviceRGB` / `DeviceGray` / `DeviceCMYK`.
    pub color_space: &'static str,
    /// `DCTDecode` or `FlateDecode`.
    pub filter: &'static str,
    pub data: Vec<u8>,
    /// Flate-compressed 8-bit alpha, when the image has transparency.
    pub smask: Option<Vec<u8>>,
}

/// A visual signature, validated and turned into drawing operations.
#[derive(Clone, Debug, PartialEq)]
pub struct Visual {
    pub kind: VisualKind,
    /// Natural size of the drawing (its own units, y up).
    pub width: f64,
    pub height: f64,
    /// Content stream operators in natural units; images are drawn as `/Im0`.
    pub content: Vec<u8>,
    pub image: Option<ImageObject>,
}

impl Visual {
    pub fn new(kind: VisualKind) -> Result<Arc<Self>, PdfError> {
        guard(|| {
            let v = match &kind {
                VisualKind::Drawn {
                    strokes,
                    width,
                    height,
                } => drawn(strokes, *width, *height)?,
                VisualKind::Typed { name } => typed(name)?,
                VisualKind::Image { data, remove_white } => image(data, *remove_white)?,
            };
            Ok(Arc::new(Self { kind, ..v }))
        })
    }

    /// Content that draws this signature fitted (aspect kept, centred) into a `w` × `h`
    /// box with the origin at its bottom-left.
    pub fn fitted(&self, w: f64, h: f64) -> Vec<u8> {
        let s = (w / self.width.max(1e-6)).min(h / self.height.max(1e-6));
        let tx = (w - self.width * s) / 2.0;
        let ty = (h - self.height * s) / 2.0;
        let mut out = format!(
            "q {} 0 0 {} {} {} cm\n",
            fmt_num(s),
            fmt_num(s),
            fmt_num(tx),
            fmt_num(ty)
        )
        .into_bytes();
        out.extend_from_slice(&self.content);
        out.extend_from_slice(b"\nQ\n");
        out
    }
}

/// Ink colour of drawn and typed signatures (blue-black).
const INK: &str = "0.08 0.12 0.35";

fn drawn(strokes: &[Vec<[f32; 2]>], width: f32, height: f32) -> Result<Visual, PdfError> {
    if strokes.is_empty() || strokes.iter().all(Vec::is_empty) {
        return Err(PdfError::Refused("the signature is empty".into()));
    }
    if strokes.len() > MAX_STROKES || strokes.iter().any(|s| s.len() > MAX_POINTS) {
        return Err(PdfError::Refused(format!(
            "a drawn signature has at most {MAX_STROKES} strokes of {MAX_POINTS} points"
        )));
    }
    if !(width.is_finite() && height.is_finite() && width >= 1.0 && height >= 1.0)
        || width > 10_000.0
        || height > 10_000.0
    {
        return Err(PdfError::Refused("invalid signature pad size".into()));
    }
    // Bounding box of the ink, so the signature is fitted by its strokes, not the pad.
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in strokes.iter().flatten() {
        if !(p[0].is_finite() && p[1].is_finite()) {
            return Err(PdfError::Refused("invalid stroke coordinates".into()));
        }
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let pen = (width.max(height) / 120.0).max(1.0) as f64;
    let margin = pen;
    let w = f64::from(x1 - x0) + 2.0 * margin;
    let h = f64::from(y1 - y0) + 2.0 * margin;
    let map = |p: [f32; 2]| -> (f64, f64) {
        (
            f64::from(p[0] - x0) + margin,
            h - (f64::from(p[1] - y0) + margin),
        )
    };
    let mut out = format!("{INK} RG {INK} rg 1 J 1 j {} w\n", fmt_num(pen));
    for stroke in strokes.iter().filter(|s| !s.is_empty()) {
        let pts: Vec<(f64, f64)> = stroke.iter().map(|p| map(*p)).collect();
        if pts.len() == 1 {
            let (x, y) = pts[0];
            out.push_str(&format!(
                "{} {} m {} {} l S\n",
                fmt_num(x),
                fmt_num(y),
                fmt_num(x + 0.01),
                fmt_num(y)
            ));
            continue;
        }
        out.push_str(&format!("{} {} m\n", fmt_num(pts[0].0), fmt_num(pts[0].1)));
        // Catmull-Rom through the points, as cubic Béziers: smooth without overshoot.
        for i in 0..pts.len() - 1 {
            let p0 = pts[i.saturating_sub(1)];
            let p1 = pts[i];
            let p2 = pts[i + 1];
            let p3 = pts[(i + 2).min(pts.len() - 1)];
            let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
            out.push_str(&format!(
                "{} {} {} {} {} {} c\n",
                fmt_num(c1.0),
                fmt_num(c1.1),
                fmt_num(c2.0),
                fmt_num(c2.1),
                fmt_num(p2.0),
                fmt_num(p2.1)
            ));
        }
        out.push_str("S\n");
    }
    Ok(Visual {
        kind: VisualKind::Typed {
            name: String::new(),
        },
        width: w.max(1.0),
        height: h.max(1.0),
        content: out.into_bytes(),
        image: None,
    })
}

struct PathOut {
    out: String,
    scale: f64,
    dx: f64,
    dy: f64,
    last: (f64, f64),
}

impl PathOut {
    fn pt(&self, x: f32, y: f32) -> (f64, f64) {
        (
            f64::from(x) * self.scale + self.dx,
            f64::from(y) * self.scale + self.dy,
        )
    }
}

impl ttf_parser::OutlineBuilder for PathOut {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.out
            .push_str(&format!("{} {} m\n", fmt_num(p.0), fmt_num(p.1)));
        self.last = p;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.pt(x, y);
        self.out
            .push_str(&format!("{} {} l\n", fmt_num(p.0), fmt_num(p.1)));
        self.last = p;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let q = self.pt(x1, y1);
        let p = self.pt(x, y);
        let l = self.last;
        let c1 = (l.0 + 2.0 / 3.0 * (q.0 - l.0), l.1 + 2.0 / 3.0 * (q.1 - l.1));
        let c2 = (p.0 + 2.0 / 3.0 * (q.0 - p.0), p.1 + 2.0 / 3.0 * (q.1 - p.1));
        self.out.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            fmt_num(c1.0),
            fmt_num(c1.1),
            fmt_num(c2.0),
            fmt_num(c2.1),
            fmt_num(p.0),
            fmt_num(p.1)
        ));
        self.last = p;
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let a = self.pt(x1, y1);
        let b = self.pt(x2, y2);
        let p = self.pt(x, y);
        self.out.push_str(&format!(
            "{} {} {} {} {} {} c\n",
            fmt_num(a.0),
            fmt_num(a.1),
            fmt_num(b.0),
            fmt_num(b.1),
            fmt_num(p.0),
            fmt_num(p.1)
        ));
        self.last = p;
    }
    fn close(&mut self) {
        self.out.push_str("h\n");
    }
}

/// Whether the signature font has a glyph for every character of `name`.
pub fn typed_supported(name: &str) -> Result<(), char> {
    let Ok(face) = ttf_parser::Face::parse(SIGNATURE_FONT, 0) else {
        return Err('?');
    };
    for ch in name.chars() {
        if ch != ' ' && face.glyph_index(ch).is_none() {
            return Err(ch);
        }
    }
    Ok(())
}

fn typed(name: &str) -> Result<Visual, PdfError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PdfError::Refused("type a name to sign with".into()));
    }
    if name.chars().count() > MAX_TYPED {
        return Err(PdfError::Refused(format!(
            "a typed signature has at most {MAX_TYPED} characters"
        )));
    }
    if let Err(ch) = typed_supported(name) {
        return Err(PdfError::Refused(format!(
            "The character “{ch}” is not available in the signature font"
        )));
    }
    let face = ttf_parser::Face::parse(SIGNATURE_FONT, 0)
        .map_err(|_| PdfError::Unsupported("the signature font".into()))?;
    let upem = f64::from(face.units_per_em().max(1));
    let size = 100.0;
    let scale = size / upem;
    let ascender = f64::from(face.ascender()) * scale;
    let descender = f64::from(face.descender()) * scale;
    let mut builder = PathOut {
        out: format!("{INK} rg\n"),
        scale,
        dx: 0.0,
        dy: -descender,
        last: (0.0, 0.0),
    };
    let mut x = 0.0;
    for ch in name.chars() {
        let Some(gid) = face.glyph_index(ch) else {
            continue;
        };
        builder.dx = x;
        face.outline_glyph(gid, &mut builder);
        x += f64::from(face.glyph_hor_advance(gid).unwrap_or(0)) * scale;
    }
    builder.out.push_str("f\n");
    Ok(Visual {
        kind: VisualKind::Typed {
            name: name.to_string(),
        },
        width: x.max(1.0),
        height: (ascender - descender).max(1.0),
        content: builder.out.into_bytes(),
        image: None,
    })
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = enc.write_all(data);
    enc.finish().unwrap_or_default()
}

fn image(data: &[u8], remove_white: bool) -> Result<Visual, PdfError> {
    use image::ImageDecoder;
    if data.len() > MAX_IMAGE_BYTES {
        return Err(PdfError::Refused(format!(
            "the image is larger than {} MiB",
            MAX_IMAGE_BYTES >> 20
        )));
    }
    let format = image::guess_format(data)
        .map_err(|_| PdfError::Refused("choose a PNG or JPEG image".into()))?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(PdfError::Refused("choose a PNG or JPEG image".into()));
    }
    let reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| PdfError::Refused("the image could not be read".into()))?;
    if u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS || w == 0 || h == 0 {
        return Err(PdfError::Refused(format!(
            "the image is larger than {} megapixels",
            MAX_IMAGE_PIXELS / 1_000_000
        )));
    }
    let content = format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q").into_bytes();
    if format == image::ImageFormat::Jpeg && !remove_white {
        let dec = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(data))
            .map_err(|_| PdfError::Refused("the image could not be read".into()))?;
        let color_space = match dec.original_color_type() {
            image::ExtendedColorType::L8 => "DeviceGray",
            image::ExtendedColorType::Cmyk8 => "DeviceCMYK",
            _ => "DeviceRGB",
        };
        return Ok(Visual {
            kind: VisualKind::Typed {
                name: String::new(),
            },
            width: f64::from(w),
            height: f64::from(h),
            content,
            image: Some(ImageObject {
                width: w,
                height: h,
                color_space,
                filter: "DCTDecode",
                data: data.to_vec(),
                smask: None,
            }),
        });
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|_| PdfError::Refused("the image could not be read".into()))?
        .to_rgba8();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    let mut transparent = false;
    for px in rgba.pixels() {
        let [r, g, b, mut a] = px.0;
        if remove_white {
            let m = r.min(g).min(b);
            if m >= 235 {
                a = 0;
            } else if m > 215 {
                a = ((u16::from(a) * u16::from(235 - m)) / 20) as u8;
            }
        }
        transparent |= a < 255;
        rgb.extend_from_slice(&[r, g, b]);
        alpha.push(a);
    }
    Ok(Visual {
        kind: VisualKind::Typed {
            name: String::new(),
        },
        width: f64::from(w),
        height: f64::from(h),
        content,
        image: Some(ImageObject {
            width: w,
            height: h,
            color_space: "DeviceRGB",
            filter: "FlateDecode",
            data: deflate(&rgb),
            smask: transparent.then(|| deflate(&alpha)),
        }),
    })
}

/// A visual signature placed on a page, movable until saved.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub id: u64,
    pub page: usize,
    /// Page-space rectangle.
    pub rect: [f32; 4],
    /// The signature field it fills, if any.
    pub field: Option<String>,
    pub visual: Arc<Visual>,
}

/// Check a page-space rectangle against a page of `size` (width, height).
pub fn check_rect(rect: [f32; 4], size: (f32, f32)) -> Result<(), PdfError> {
    let [x0, y0, x1, y1] = rect;
    if !rect.iter().all(|v| v.is_finite()) || x1 - x0 < 4.0 || y1 - y0 < 4.0 {
        return Err(PdfError::Refused(
            "the rectangle is too small or invalid".into(),
        ));
    }
    if x0 < -0.5 || y0 < -0.5 || x1 > size.0 + 0.5 || y1 > size.1 + 0.5 {
        return Err(PdfError::Refused(
            "the rectangle is outside the page".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawn_strokes_become_paths() {
        let v = Visual::new(VisualKind::Drawn {
            strokes: vec![
                vec![[10.0, 10.0], [50.0, 40.0], [90.0, 10.0]],
                vec![[20.0, 30.0]],
            ],
            width: 300.0,
            height: 100.0,
        })
        .expect("drawn");
        let s = String::from_utf8_lossy(&v.content);
        assert!(s.contains(" c\n") && s.contains("S\n"), "{s}");
        assert!(v.width > 80.0 && v.height > 30.0);
        assert!(
            Visual::new(VisualKind::Drawn {
                strokes: vec![],
                width: 10.0,
                height: 10.0
            })
            .is_err()
        );
        assert!(
            Visual::new(VisualKind::Drawn {
                strokes: vec![vec![[0.0, 0.0]]; MAX_STROKES + 1],
                width: 10.0,
                height: 10.0
            })
            .is_err()
        );
        assert!(
            Visual::new(VisualKind::Drawn {
                strokes: vec![vec![[0.0, 0.0]; MAX_POINTS + 1]],
                width: 10.0,
                height: 10.0
            })
            .is_err()
        );
    }

    #[test]
    fn typed_names_become_outlines_without_a_font() {
        let v = Visual::new(VisualKind::Typed {
            name: "Jane Doe".into(),
        })
        .expect("typed");
        let s = String::from_utf8_lossy(&v.content);
        assert!(s.contains(" m\n") && s.ends_with("f\n"));
        assert!(!s.contains("Tf"), "no text operators: vector outlines only");
        assert!(v.width > v.height);
        assert!(typed_supported("Zoë Łukasz").is_ok());
        assert!(Visual::new(VisualKind::Typed { name: "  ".into() }).is_err());
        assert!(
            Visual::new(VisualKind::Typed {
                name: "日本".into()
            })
            .is_err()
        );
    }

    #[test]
    fn images_are_bounded() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 255, 255, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .expect("png");
        let v = Visual::new(VisualKind::Image {
            data: png.clone(),
            remove_white: true,
        })
        .expect("image");
        let im = v.image.as_ref().expect("xobject");
        assert_eq!((im.width, im.height, im.filter), (4, 2, "FlateDecode"));
        assert!(im.smask.is_some(), "white became transparent");
        let v = Visual::new(VisualKind::Image {
            data: png,
            remove_white: false,
        })
        .expect("image");
        assert!(v.image.as_ref().is_some_and(|i| i.smask.is_none()));
        let big = image::RgbImage::new(2001, 2001);
        let mut data = Vec::new();
        big.write_to(
            &mut std::io::Cursor::new(&mut data),
            image::ImageFormat::Png,
        )
        .expect("png");
        let err = Visual::new(VisualKind::Image {
            data,
            remove_white: false,
        })
        .expect_err("too many pixels");
        assert!(err.to_string().contains("megapixels"));
        assert!(
            Visual::new(VisualKind::Image {
                data: b"GIF89a".to_vec(),
                remove_white: false
            })
            .is_err()
        );
    }

    #[test]
    fn rectangles_are_checked() {
        assert!(check_rect([10.0, 10.0, 100.0, 50.0], (595.0, 842.0)).is_ok());
        assert!(check_rect([10.0, 10.0, 700.0, 50.0], (595.0, 842.0)).is_err());
        assert!(check_rect([10.0, 10.0, 11.0, 50.0], (595.0, 842.0)).is_err());
        assert!(check_rect([f32::NAN, 10.0, 100.0, 50.0], (595.0, 842.0)).is_err());
    }
}
