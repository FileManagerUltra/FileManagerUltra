//! Rasterize an SVG for the Markdown preview. Text, embedded raster images and external
//! resources are not supported (the features are compiled out), so an SVG can read nothing
//! from disk or the network.

use resvg::{tiny_skia, usvg};

use crate::thumbs::Thumb;

/// Whether `bytes` look like an SVG document (an `<svg` element near the start).
pub fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    head.windows(4).any(|w| w == b"<svg")
}

/// Render at the document's own size, scaled down so the longest edge is at most `max_edge`.
pub fn rasterize(bytes: &[u8], max_edge: u32) -> Result<Thumb, String> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|_| "The SVG could not be read")?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return Err("The SVG has no size".into());
    }
    let scale = (max_edge as f32 / w.max(h)).min(1.0);
    let (pw, ph) = (
        ((w * scale).round() as u32).max(1),
        ((h * scale).round() as u32).max(1),
    );
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("The SVG has no size")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let thumb = Thumb {
        width: pw,
        height: ph,
        rgba: unpremultiply(pixmap.data()),
    };
    thumb
        .valid()
        .then_some(thumb)
        .ok_or("The SVG has no size".into())
}

fn unpremultiply(data: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    for px in out.as_chunks_mut::<4>().0 {
        let a = u32::from(px[3]);
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#ff0000"/></svg>"##;

    #[test]
    fn renders_and_scales_down() {
        assert!(looks_like_svg(SVG));
        let t = rasterize(SVG, 100).expect("svg");
        assert_eq!((t.width, t.height), (40, 20));
        assert_eq!(&t.rgba[..4], &[255, 0, 0, 255]);
        let small = rasterize(SVG, 10).expect("svg");
        assert_eq!((small.width, small.height), (10, 5));
    }

    #[test]
    fn hostile_input_is_an_error() {
        assert!(rasterize(b"<svg", 100).is_err());
        assert!(rasterize(b"\x00\x01", 100).is_err());
        let huge = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1e30" height="1e30"/>"#;
        let _ = rasterize(huge, 64);
        let external = br#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="8" height="8"><image href="/etc/passwd" width="8" height="8"/></svg>"#;
        assert!(rasterize(external, 64).is_ok());
    }
}
