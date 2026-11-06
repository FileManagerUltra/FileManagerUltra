//! Raster images: sizes are checked from the header before any pixel is decoded.

use std::io::Cursor;

use fm_vfs::{Entry, FmError, Vfs};
use image::{AnimationDecoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::{ImageDoc, Limits, MetaDoc, Preview, base_meta, text};

const ANIM_EDGE: u32 = 800;
const ANIM_BYTES: usize = 96 << 20;

fn error_card(entry: &Entry, why: String) -> Preview {
    let mut rows = base_meta(entry);
    rows.push(("Image".into(), why));
    Preview::Meta(MetaDoc {
        rows,
        hint: Some("The image was not decoded.".into()),
        poster: None,
        video: None,
    })
}

fn fit(img: DynamicImage, edge: u32) -> DynamicImage {
    if img.width() <= edge && img.height() <= edge {
        return img;
    }
    let scale = f64::from(edge) / f64::from(img.width().max(img.height()));
    let w = ((f64::from(img.width()) * scale).round() as u32).max(1);
    let h = ((f64::from(img.height()) * scale).round() as u32).max(1);
    resize_simd(&img, w, h).unwrap_or_else(|| img.thumbnail(edge, edge))
}

/// SIMD resize; `None` makes the caller fall back to the (slower) `image` filter.
fn resize_simd(img: &DynamicImage, w: u32, h: u32) -> Option<DynamicImage> {
    use fast_image_resize::{
        FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer, images::Image,
    };
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    let mut resizer = Resizer::new();
    if img.color().has_alpha() {
        let rgba = img.to_rgba8();
        let (sw, sh) = (rgba.width(), rgba.height());
        let mut src = Image::from_vec_u8(sw, sh, rgba.into_raw(), PixelType::U8x4).ok()?;
        let mut dst = Image::new(w, h, PixelType::U8x4);
        let mul = fast_image_resize::MulDiv::default();
        mul.multiply_alpha_inplace(&mut src).ok()?;
        resizer.resize(&src, &mut dst, &opts).ok()?;
        mul.divide_alpha_inplace(&mut dst).ok()?;
        image::RgbaImage::from_raw(w, h, dst.into_vec()).map(DynamicImage::ImageRgba8)
    } else {
        let rgb = img.to_rgb8();
        let (sw, sh) = (rgb.width(), rgb.height());
        let src = Image::from_vec_u8(sw, sh, rgb.into_raw(), PixelType::U8x3).ok()?;
        let mut dst = Image::new(w, h, PixelType::U8x3);
        resizer.resize(&src, &mut dst, &opts).ok()?;
        image::RgbImage::from_raw(w, h, dst.into_vec()).map(DynamicImage::ImageRgb8)
    }
}

fn name(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::Gif => "GIF",
        ImageFormat::WebP => "WebP",
        ImageFormat::Bmp => "BMP",
        ImageFormat::Ico => "ICO",
        _ => "image",
    }
}

pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<Preview, FmError> {
    if entry.size.is_some_and(|s| s > limits.image_bytes) {
        return Ok(error_card(entry, "file is too large to preview".into()));
    }
    let (bytes, more) = text::read_capped(vfs, entry, limits.image_bytes as usize)?;
    if more {
        return Ok(error_card(entry, "file is too large to preview".into()));
    }
    Ok(decode(entry, &bytes, limits))
}

pub fn decode(entry: &Entry, bytes: &[u8], limits: &Limits) -> Preview {
    decode_with(entry, bytes, limits, true)
}

/// Like [`decode`] but never decodes animation frames (thumbnails want the first frame only).
pub fn decode_still(entry: &Entry, bytes: &[u8], limits: &Limits) -> Preview {
    decode_with(entry, bytes, limits, false)
}

/// Full-resolution RGBA of the first frame, for the clipboard. The header is checked against
/// `max_pixels` before any pixel is decoded.
pub fn decode_original(bytes: &[u8], max_pixels: u64) -> Result<(u32, u32, Vec<u8>), String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format().ok_or("unknown image format")?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|e| format!("unreadable header: {e}"))?;
    if u64::from(w) * u64::from(h) > max_pixels {
        return Err(format!("{w}×{h} px is too large to copy"));
    }
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(512 << 20);
    lim.max_image_width = Some(w);
    lim.max_image_height = Some(h);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(lim);
    let rgba = reader
        .decode()
        .map_err(|e| format!("could not decode: {e}"))?
        .to_rgba8();
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn decode_with(entry: &Entry, bytes: &[u8], limits: &Limits, animate: bool) -> Preview {
    let reader = match ImageReader::new(Cursor::new(bytes)).with_guessed_format() {
        Ok(r) => r,
        Err(e) => return error_card(entry, e.to_string()),
    };
    let Some(format) = reader.format() else {
        return error_card(entry, "unknown image format".into());
    };
    let (w, h) = match reader.into_dimensions() {
        Ok(d) => d,
        Err(e) => return error_card(entry, format!("unreadable header: {e}")),
    };
    if u64::from(w) * u64::from(h) > limits.image_pixels {
        return error_card(entry, format!("{w}×{h} px exceeds the preview limit"));
    }
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(512 << 20);
    lim.max_image_width = Some(w);
    lim.max_image_height = Some(h);

    if animate
        && format == ImageFormat::Gif
        && let Some(doc) = gif(bytes, (w, h), lim.clone(), limits)
        && doc.frames.len() > 1
    {
        return Preview::Image(doc);
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(lim);
    let img = match reader.decode() {
        Ok(i) => fit(i, limits.image_edge),
        Err(e) => return error_card(entry, format!("could not decode: {e}")),
    };
    let rgba = img.to_rgba8();
    Preview::Image(ImageDoc {
        width: rgba.width(),
        height: rgba.height(),
        frames: vec![(rgba.into_raw(), 0)],
        original: (w, h),
        format: name(format),
    })
}

fn gif(bytes: &[u8], orig: (u32, u32), lim: image::Limits, limits: &Limits) -> Option<ImageDoc> {
    let mut dec = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)).ok()?;
    dec.set_limits(lim).ok()?;
    let mut frames = Vec::new();
    let mut size = (0, 0);
    let mut total = 0usize;
    for f in dec.into_frames().take(limits.gif_frames) {
        let f = f.ok()?;
        let (n, d) = f.delay().numer_denom_ms();
        let delay = (n / d.max(1)).clamp(20, 10_000);
        let img = fit(DynamicImage::ImageRgba8(f.into_buffer()), ANIM_EDGE);
        let rgba = img.to_rgba8();
        size = (rgba.width(), rgba.height());
        total += rgba.as_raw().len();
        frames.push((rgba.into_raw(), delay));
        if total > ANIM_BYTES {
            break;
        }
    }
    Some(ImageDoc {
        width: size.0,
        height: size.1,
        frames,
        original: orig,
        format: "GIF",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::{EntryKind, VfsUri};

    fn entry() -> Entry {
        Entry::new(VfsUri::parse("/x.png").expect("uri"), EntryKind::File)
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([1, 2, 3, 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("png");
        out
    }

    #[test]
    fn decodes_and_downscales() {
        let lim = Limits {
            image_edge: 8,
            ..Limits::default()
        };
        let Preview::Image(d) = decode(&entry(), &png(32, 16), &lim) else {
            panic!("image")
        };
        assert_eq!((d.width, d.height), (8, 4));
        assert_eq!(d.original, (32, 16));
        assert_eq!(d.frames[0].0.len(), 8 * 4 * 4);
    }

    #[test]
    fn pixel_bomb_is_refused_from_the_header() {
        let lim = Limits {
            image_pixels: 100,
            ..Limits::default()
        };
        assert!(matches!(
            decode(&entry(), &png(64, 64), &lim),
            Preview::Meta(_)
        ));
    }

    #[test]
    fn garbage_and_truncated_are_cards() {
        assert!(matches!(
            decode(&entry(), b"not an image", &Limits::default()),
            Preview::Meta(_)
        ));
        let p = png(16, 16);
        assert!(matches!(
            decode(&entry(), &p[..p.len() / 2], &Limits::default()),
            Preview::Meta(_)
        ));
    }
}
