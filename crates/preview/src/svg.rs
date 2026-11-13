//! Rasterize SVGs for the SVG preview and for Markdown images. Text is drawn with the
//! installed system fonts. `<image>` references (files, URLs and `data:` payloads) are
//! refused, so an SVG can read nothing from disk or the network besides those fonts.

use std::collections::HashSet;
use std::sync::{Arc, OnceLock};

use fm_vfs::{Entry, FmError, Vfs};
use resvg::{
    tiny_skia,
    usvg::{self, fontdb},
};

use crate::{ImageDoc, Limits, StructDoc, SvgDoc, structured, text, thumbs::Thumb};

/// Larger files are shown as source only: parsing them could take seconds.
pub const MAX_BYTES: usize = 16 << 20;
/// A declared width or height above this is treated as broken rather than drawn.
const MAX_DECLARED: f32 = 1_000_000.0;

/// Gzip data: an `.svgz` file.
pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Decompress at most `cap` bytes of gzip data. Returns what was decoded (for the source
/// view) and whether it is complete; a small file can never expand past `cap` in memory.
pub fn inflate(bytes: &[u8], cap: usize) -> (Vec<u8>, Result<(), String>) {
    use std::io::Read;
    let mut out = Vec::new();
    let read = flate2::read::GzDecoder::new(bytes)
        .take(cap as u64 + 1)
        .read_to_end(&mut out);
    if out.len() > cap {
        out.truncate(cap);
        return (
            out,
            Err(format!(
                "The SVG is larger than {} MiB once decompressed, so only the start of its source is shown",
                cap >> 20
            )),
        );
    }
    match read {
        Ok(_) => (out, Ok(())),
        Err(_) => (out, Err("The compressed SVG is damaged".into())),
    }
}

/// The SVG text of a file read up to `cap` bytes: decompressed when gzip. `Err` explains
/// why it cannot be drawn; the bytes are still what the source view can show.
fn svg_bytes(bytes: Vec<u8>, more: bool, cap: usize) -> (Vec<u8>, bool, Result<(), String>) {
    if more {
        let why = format!(
            "The SVG is larger than {} MiB, so only its source is shown",
            cap >> 20
        );
        if is_gzip(&bytes) {
            let (text, _) = inflate(&bytes, cap);
            return (text, true, Err(why));
        }
        return (bytes, true, Err(why));
    }
    if is_gzip(&bytes) {
        let (text, status) = inflate(&bytes, cap);
        let cut = status.is_err();
        return (text, cut, status);
    }
    (bytes, false, Ok(()))
}

/// Read an SVG file once and build both the drawing and the highlighted source.
pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<SvgDoc, FmError> {
    let cap = MAX_BYTES.min(usize::try_from(limits.image_bytes).unwrap_or(MAX_BYTES));
    let (raw, more) = text::read_capped(vfs, entry, cap)?;
    let compressed = is_gzip(&raw);
    let (bytes, more, status) =
        svg_bytes(raw, more || entry.size.is_some_and(|s| s > cap as u64), cap);
    let total = if compressed {
        bytes.len() as u64
    } else {
        entry.size.unwrap_or(bytes.len() as u64)
    };
    let image = status.and_then(|()| draw(&bytes, limits));
    let shown = bytes.len().min(limits.text_bytes);
    let source: StructDoc = structured::doc(
        bytes.get(..shown).unwrap_or_default(),
        more || shown < bytes.len(),
        total,
        limits,
        structured::Format::Xml,
    );
    Ok(SvgDoc { image, source })
}

/// A thumbnail whose longest edge is `edge`: small icons are drawn larger, not stretched.
pub fn thumbnail(vfs: &dyn Vfs, entry: &Entry, edge: u32) -> Option<Thumb> {
    let (raw, more) = text::read_capped(vfs, entry, MAX_BYTES).ok()?;
    let (bytes, _, status) = svg_bytes(raw, more, MAX_BYTES);
    status.ok()?;
    let limits = Limits {
        image_edge: edge,
        ..Limits::default()
    };
    let doc = draw(&bytes, &limits).ok()?;
    let (rgba, _) = doc.frames.into_iter().next()?;
    let thumb = Thumb {
        width: doc.width,
        height: doc.height,
        rgba,
    };
    thumb.valid().then_some(thumb)
}

/// Draw `bytes` so the longest edge is `limits.image_edge` pixels (scaling up small icons and
/// down large drawings); `original` keeps the declared size for the 100% view.
pub fn draw(bytes: &[u8], limits: &Limits) -> Result<ImageDoc, String> {
    let tree = parse(bytes)?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if w > MAX_DECLARED || h > MAX_DECLARED {
        return Err(format!("The SVG declares an unusable size ({w}×{h})"));
    }
    let edge = limits.image_edge.max(1) as f32;
    let mut scale = edge / w.max(h);
    let pixels = f64::from(w * scale) * f64::from(h * scale);
    if pixels > limits.image_pixels as f64 {
        scale *= (limits.image_pixels as f64 / pixels).sqrt() as f32;
    }
    let thumb = paint(&tree, scale)?;
    Ok(ImageDoc {
        width: thumb.width,
        height: thumb.height,
        frames: vec![(thumb.rgba, 0)],
        original: ((w.round() as u32).max(1), (h.round() as u32).max(1)),
        format: "svg",
    })
}

/// System fonts, loaded once (on a worker thread, on first use), with generic families
/// pointed at installed fonts. The default serif family doubles as usvg's last fallback, so
/// text asking for an absent font (Arial on Linux) is still drawn.
struct Fonts {
    db: Arc<fontdb::Database>,
    serif: String,
    /// Lower-case names of every installed family.
    families: HashSet<String>,
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let installed = |candidates: &[&str]| {
            candidates
                .iter()
                .find(|name| {
                    db.faces().any(|face| {
                        face.families
                            .iter()
                            .any(|(family, _)| family.eq_ignore_ascii_case(name))
                    })
                })
                .map(|name| (*name).to_string())
        };
        let any = db
            .faces()
            .find_map(|face| face.families.first().map(|(family, _)| family.clone()));
        let sans = installed(&[
            "DejaVu Sans",
            "Liberation Sans",
            "Noto Sans",
            "Arial",
            "Helvetica",
            "Segoe UI",
            "Ubuntu",
            "Cantarell",
        ])
        .or_else(|| any.clone())
        .unwrap_or_else(|| "Arial".into());
        let serif = installed(&[
            "Times New Roman",
            "Liberation Serif",
            "DejaVu Serif",
            "Noto Serif",
            "Times",
        ])
        .unwrap_or_else(|| sans.clone());
        let mono = installed(&[
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Noto Sans Mono",
            "Courier New",
            "Menlo",
            "Consolas",
            "Ubuntu Mono",
        ])
        .unwrap_or_else(|| sans.clone());
        db.set_serif_family(serif.clone());
        db.set_sans_serif_family(sans.clone());
        db.set_monospace_family(mono);
        db.set_cursive_family(sans.clone());
        db.set_fantasy_family(sans);
        let families = db
            .faces()
            .flat_map(|face| {
                face.families
                    .iter()
                    .map(|(family, _)| family.to_lowercase())
            })
            .collect();
        Fonts {
            db: Arc::new(db),
            serif,
            families,
        }
    })
}

/// The generic family that best stands in for a font that is not installed: SVGs from
/// Windows, macOS and diagram tools name Helvetica, Segoe UI, Consolas and the like.
fn stand_in(name: &str) -> fontdb::Family<'static> {
    let name = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&[
        "mono", "courier", "consola", "menlo", "monaco", "code", "terminal",
    ]) {
        fontdb::Family::Monospace
    } else if !name.contains("sans")
        && has(&[
            "serif", "times", "georgia", "garamond", "cambria", "palatino", "book", "roman",
        ])
    {
        fontdb::Family::Serif
    } else {
        fontdb::Family::SansSerif
    }
}

/// usvg's own selector, except that an absent named font falls back to its stand-in before
/// the final serif fallback (instead of every absent font turning serif).
fn select_font(font: &usvg::Font, db: &mut Arc<fontdb::Database>) -> Option<fontdb::ID> {
    let installed = &fonts().families;
    let mut names = Vec::new();
    let mut stand_ins = Vec::new();
    for family in font.families() {
        names.push(match family {
            usvg::FontFamily::Serif => fontdb::Family::Serif,
            usvg::FontFamily::SansSerif => fontdb::Family::SansSerif,
            usvg::FontFamily::Cursive => fontdb::Family::Cursive,
            usvg::FontFamily::Fantasy => fontdb::Family::Fantasy,
            usvg::FontFamily::Monospace => fontdb::Family::Monospace,
            usvg::FontFamily::Named(name) => {
                if !installed.contains(&name.to_lowercase()) {
                    stand_ins.push(stand_in(name));
                }
                fontdb::Family::Name(name)
            }
        });
    }
    names.extend(stand_ins.into_iter().take(1));
    names.push(fontdb::Family::Serif);
    let stretch = match font.stretch() {
        usvg::FontStretch::UltraCondensed => fontdb::Stretch::UltraCondensed,
        usvg::FontStretch::ExtraCondensed => fontdb::Stretch::ExtraCondensed,
        usvg::FontStretch::Condensed => fontdb::Stretch::Condensed,
        usvg::FontStretch::SemiCondensed => fontdb::Stretch::SemiCondensed,
        usvg::FontStretch::Normal => fontdb::Stretch::Normal,
        usvg::FontStretch::SemiExpanded => fontdb::Stretch::SemiExpanded,
        usvg::FontStretch::Expanded => fontdb::Stretch::Expanded,
        usvg::FontStretch::ExtraExpanded => fontdb::Stretch::ExtraExpanded,
        usvg::FontStretch::UltraExpanded => fontdb::Stretch::UltraExpanded,
    };
    let style = match font.style() {
        usvg::FontStyle::Normal => fontdb::Style::Normal,
        usvg::FontStyle::Italic => fontdb::Style::Italic,
        usvg::FontStyle::Oblique => fontdb::Style::Oblique,
    };
    db.query(&fontdb::Query {
        families: &names,
        weight: fontdb::Weight(font.weight()),
        stretch,
        style,
    })
}

fn options() -> usvg::Options<'static> {
    let fonts = fonts();
    usvg::Options {
        fontdb: fonts.db.clone(),
        // Like browsers: text without a font-family is serif.
        font_family: fonts.serif.clone(),
        font_resolver: usvg::FontResolver {
            select_font: Box::new(select_font),
            select_fallback: usvg::FontResolver::default_fallback_selector(),
        },
        // Never follow `<image href>`: the default resolver reads any local path.
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        resources_dir: None,
        ..usvg::Options::default()
    }
}

fn parse(bytes: &[u8]) -> Result<usvg::Tree, String> {
    let opt = options();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|_| "The SVG could not be read")?;
    let size = tree.size();
    let (w, h) = (size.width(), size.height());
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return Err("The SVG has no size".into());
    }
    Ok(tree)
}

fn paint(tree: &usvg::Tree, scale: f32) -> Result<Thumb, String> {
    let size = tree.size();
    let (pw, ph) = (
        ((size.width() * scale).round() as u32).max(1),
        ((size.height() * scale).round() as u32).max(1),
    );
    let mut pixmap = tiny_skia::Pixmap::new(pw, ph).ok_or("The SVG has no size")?;
    resvg::render(
        tree,
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

/// Whether `bytes` look like an SVG document (an `<svg` element near the start).
pub fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(2048)];
    head.windows(4).any(|w| w == b"<svg")
}

/// Render at the document's own size, scaled down so the longest edge is at most `max_edge`.
pub fn rasterize(bytes: &[u8], max_edge: u32) -> Result<Thumb, String> {
    let tree = parse(bytes)?;
    let size = tree.size();
    paint(
        &tree,
        (max_edge as f32 / size.width().max(size.height())).min(1.0),
    )
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
    fn draw_fills_the_preview_edge_and_keeps_the_declared_size() {
        let limits = Limits::default();
        let icon = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="8"><rect width="16" height="8" fill="#00ff00"/></svg>"##;
        let doc = draw(icon, &limits).expect("icon");
        assert_eq!(
            (doc.width, doc.height),
            (2048, 1024),
            "small icons are drawn large"
        );
        assert_eq!(doc.original, (16, 8));
        assert_eq!(doc.format, "svg");
        let pixels = &doc.frames.first().expect("frame").0;
        assert_eq!(&pixels[..4], &[0, 255, 0, 255]);

        let tight = Limits {
            image_edge: 4096,
            image_pixels: 1_000_000,
            ..Limits::default()
        };
        let doc = draw(icon, &tight).expect("capped");
        assert!(u64::from(doc.width) * u64::from(doc.height) <= 1_000_000);

        let huge = br#"<svg xmlns="http://www.w3.org/2000/svg" width="1e30" height="1e30"/>"#;
        assert!(draw(huge, &limits).is_err());
        assert!(draw(b"<svg", &limits).is_err());
        let thin = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="1"/>"#;
        let doc = draw(thin, &limits).expect("thin");
        assert_eq!((doc.width, doc.height), (2048, 1));
    }

    #[test]
    fn render_keeps_the_source_and_reports_failures() {
        use fm_testkit::MemFs;
        use fm_vfs::VfsUri;
        let vfs = MemFs::new();
        let stat = |path: &str| vfs.stat(&VfsUri::parse(path).expect("uri")).expect("stat");
        vfs.put_str("/a.svg", SVG).expect("file");
        let doc = render(&vfs, &stat("/a.svg"), &Limits::default()).expect("render");
        assert!(doc.image.is_ok());
        assert!(!doc.source.raw.lines.is_empty());
        assert!(doc.source.pretty.is_ok());

        vfs.put_str("/b.svg", b"<svg><g>").expect("file");
        let doc = render(&vfs, &stat("/b.svg"), &Limits::default()).expect("render");
        assert!(doc.image.is_err());
        assert!(
            !doc.source.raw.lines.is_empty(),
            "the source is still shown"
        );

        let small = Limits {
            image_bytes: 16,
            ..Limits::default()
        };
        let doc = render(&vfs, &stat("/a.svg"), &small).expect("render");
        assert!(
            doc.image
                .expect_err("too large")
                .contains("only its source")
        );
    }

    #[test]
    fn text_is_drawn_with_system_fonts() {
        let blank = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60"/>"#;
        let label = br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="60"><text x="10" y="40" font-family="Arial, sans-serif" font-size="32" fill="black">Label</text></svg>"#;
        let ink = |bytes: &[u8]| {
            rasterize(bytes, 200)
                .expect("svg")
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|px| px[3] > 0)
                .count()
        };
        assert_eq!(ink(blank), 0);
        if fonts().db.is_empty() {
            eprintln!("no system fonts installed; skipping the text check");
            return;
        }
        assert!(ink(label) > 100, "the label is drawn");
    }

    #[test]
    fn absent_fonts_stand_in_by_kind() {
        for (name, family) in [
            ("Helvetica", fontdb::Family::SansSerif),
            ("Segoe UI", fontdb::Family::SansSerif),
            ("Open Sans", fontdb::Family::SansSerif),
            ("PT Sans Narrow", fontdb::Family::SansSerif),
            ("Times New Roman", fontdb::Family::Serif),
            ("Georgia", fontdb::Family::Serif),
            ("Noto Serif CJK", fontdb::Family::Serif),
            ("Consolas", fontdb::Family::Monospace),
            ("Fira Code", fontdb::Family::Monospace),
            ("Courier", fontdb::Family::Monospace),
            ("SomethingElse", fontdb::Family::SansSerif),
        ] {
            assert_eq!(stand_in(name), family, "{name}");
        }
    }

    #[test]
    fn image_references_are_never_read() {
        let dir = std::env::temp_dir().join(format!("fm-svg-ref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let inner = dir.join("inner.svg");
        std::fs::write(
            &inner,
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#ff0000"/></svg>"##,
        )
        .expect("inner");
        let outer = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="{}" width="8" height="8"/><image href="data:image/svg+xml;utf8,&lt;svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'&gt;&lt;rect width='8' height='8'/&gt;&lt;/svg&gt;" width="8" height="8"/></svg>"#,
            inner.display()
        );
        let thumb = rasterize(outer.as_bytes(), 8).expect("svg");
        assert!(
            thumb.rgba.as_chunks::<4>().0.iter().all(|px| px[3] == 0),
            "neither the local file nor the data URL was drawn"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).expect("gzip");
        enc.finish().expect("gzip")
    }

    #[test]
    fn svgz_is_decompressed_with_a_cap() {
        use fm_testkit::MemFs;
        use fm_vfs::VfsUri;
        let vfs = MemFs::new();
        let stat = |path: &str| vfs.stat(&VfsUri::parse(path).expect("uri")).expect("stat");
        vfs.put_str("/a.svgz", &gzip(SVG)).expect("file");
        let doc = render(&vfs, &stat("/a.svgz"), &Limits::default()).expect("render");
        assert_eq!(doc.image.expect("drawn").original, (40, 20));
        assert!(
            doc.source.pretty.is_ok(),
            "the source is the decompressed XML"
        );
        let thumb = thumbnail(&vfs, &stat("/a.svgz"), 160).expect("thumb");
        assert_eq!((thumb.width, thumb.height), (160, 80));

        // 64 MiB of zeros compresses to about 64 KiB: decoding stops at the cap.
        let bomb = gzip(&vec![b' '; 64 << 20]);
        assert!(bomb.len() < 1 << 20);
        let (out, status) = inflate(&bomb, 1 << 20);
        assert_eq!(out.len(), 1 << 20);
        assert!(status.expect_err("too large").contains("decompressed"));
        vfs.put_str("/bomb.svgz", &bomb).expect("file");
        let doc = render(&vfs, &stat("/bomb.svgz"), &Limits::default()).expect("render");
        assert!(doc.image.is_err());
        assert!(thumbnail(&vfs, &stat("/bomb.svgz"), 160).is_none());

        let mut damaged = gzip(SVG);
        damaged.truncate(damaged.len() / 2);
        let (_, status) = inflate(&damaged, 1 << 20);
        assert!(status.expect_err("damaged").contains("damaged"));
    }

    #[test]
    fn thumbnails_fill_the_edge() {
        use fm_testkit::MemFs;
        use fm_vfs::VfsUri;
        let vfs = MemFs::new();
        vfs.put_str("/a.svg", SVG).expect("file");
        let entry = vfs
            .stat(&VfsUri::parse("/a.svg").expect("uri"))
            .expect("stat");
        let thumb = thumbnail(&vfs, &entry, 256).expect("thumb");
        assert_eq!(
            (thumb.width, thumb.height),
            (256, 128),
            "a 40x20 drawing fills 256 px"
        );
        vfs.put_str("/b.svg", b"<svg").expect("file");
        let entry = vfs
            .stat(&VfsUri::parse("/b.svg").expect("uri"))
            .expect("stat");
        assert!(thumbnail(&vfs, &entry, 256).is_none());
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
