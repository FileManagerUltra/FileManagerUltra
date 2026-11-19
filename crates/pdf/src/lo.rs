//! Small helpers over `lopdf` shared by the editing modules: dereferencing, inherited
//! page attributes and the page-space transform (the same one hayro uses).

use hayro::kurbo::{Affine, Point};
use lopdf::{Dictionary, Document, Object, ObjectId};

use crate::PdfError;
use crate::strings::decode_text;

/// Follow references (bounded) to the object they name.
pub fn deref<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    let mut cur = obj;
    for _ in 0..8 {
        match cur {
            Object::Reference(id) => cur = doc.get_object(*id).ok()?,
            other => return Some(other),
        }
    }
    None
}

pub fn dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match deref(doc, obj)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

pub fn get<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    deref(doc, d.get(key).ok()?)
}

pub fn get_dict<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Dictionary> {
    dict(doc, d.get(key).ok()?)
}

pub fn get_name(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<String> {
    match get(doc, d, key)? {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

pub fn get_text(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<String> {
    match get(doc, d, key)? {
        Object::String(s, _) => Some(decode_text(s)),
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

pub fn get_num(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<f64> {
    match get(doc, d, key)? {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

pub fn num(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(f64::from(*r)),
        _ => None,
    }
}

pub fn get_rect(doc: &Document, d: &Dictionary, key: &[u8]) -> Option<[f64; 4]> {
    let Object::Array(a) = get(doc, d, key)? else {
        return None;
    };
    if a.len() != 4 {
        return None;
    }
    let mut r = [0.0; 4];
    for (i, o) in a.iter().enumerate() {
        r[i] = num(deref(doc, o)?)?;
        if !r[i].is_finite() {
            return None;
        }
    }
    Some([
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ])
}

/// A page attribute, looked up through `/Parent` (bounded).
pub fn inherited<'a>(doc: &'a Document, page: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let mut cur = page;
    for _ in 0..32 {
        if let Some(v) = get(doc, cur, key) {
            return Some(v);
        }
        cur = get_dict(doc, cur, b"Parent")?;
    }
    None
}

/// Page index → object id, in document order.
pub fn page_ids(doc: &Document) -> Vec<ObjectId> {
    doc.get_pages().into_values().collect()
}

/// Page geometry: user space → page space (top-left origin, y down, rotation applied),
/// and the displayed size. Mirrors hayro's `initial_transform(true)`.
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub to_page: Affine,
    pub width: f64,
    pub height: f64,
}

impl Geometry {
    pub fn to_user(self) -> Affine {
        self.to_page.inverse()
    }

    /// A page-space rectangle in user space `[x0, y0, x1, y1]` (normalized).
    pub fn rect_to_user(&self, r: [f32; 4]) -> [f64; 4] {
        let inv = self.to_user();
        let a = inv * Point::new(f64::from(r[0]), f64::from(r[1]));
        let b = inv * Point::new(f64::from(r[2]), f64::from(r[3]));
        [a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y)]
    }

    pub fn rect_to_page(&self, r: [f64; 4]) -> [f32; 4] {
        let a = self.to_page * Point::new(r[0], r[1]);
        let b = self.to_page * Point::new(r[2], r[3]);
        [
            a.x.min(b.x) as f32,
            a.y.min(b.y) as f32,
            a.x.max(b.x) as f32,
            a.y.max(b.y) as f32,
        ]
    }
}

pub fn geometry(doc: &Document, page_id: ObjectId) -> Result<Geometry, PdfError> {
    let page = doc
        .get_dictionary(page_id)
        .map_err(|_| PdfError::Invalid("a page is missing".into()))?;
    let rect_of = |key: &[u8]| -> Option<[f64; 4]> {
        let Object::Array(a) = inherited(doc, page, key)? else {
            return None;
        };
        if a.len() != 4 {
            return None;
        }
        let mut r = [0.0; 4];
        for (i, o) in a.iter().enumerate() {
            r[i] = num(deref(doc, o)?)?;
        }
        Some([
            r[0].min(r[2]),
            r[1].min(r[3]),
            r[0].max(r[2]),
            r[1].max(r[3]),
        ])
    };
    let media = rect_of(b"MediaBox").unwrap_or([0.0, 0.0, 595.0, 842.0]);
    let crop = rect_of(b"CropBox").unwrap_or(media);
    let crop = [
        crop[0].max(media[0]),
        crop[1].max(media[1]),
        crop[2].min(media[2]),
        crop[3].min(media[3]),
    ];
    let (bw, bh) = {
        let (w, h) = (crop[2] - crop[0], crop[3] - crop[1]);
        if w < 1e-3 || h < 1e-3 {
            (595.0, 842.0)
        } else {
            (w.max(1.0), h.max(1.0))
        }
    };
    let rotate = inherited(doc, page, b"Rotate")
        .and_then(num)
        .map_or(0, |r| (r as i64).rem_euclid(360));
    let (w, h) = if rotate == 90 || rotate == 270 {
        (bh, bw)
    } else {
        (bw, bh)
    };
    let rotation = match rotate {
        90 => Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]) * Affine::translate((0.0, -w)),
        270 => Affine::translate((0.0, h)) * Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]),
        180 => Affine::scale(-1.0) * Affine::translate((-w, -h)),
        _ => Affine::IDENTITY,
    };
    let invert = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, bh]);
    let to_page = rotation * invert * Affine::translate((-crop[0], -crop[1]));
    Ok(Geometry {
        to_page,
        width: w,
        height: h,
    })
}

/// Write a PDF number compactly.
pub fn fmt_num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() {
        "0".into()
    } else {
        s.to_string()
    }
}

/// A PDF literal string body with `(`, `)` and `\` escaped.
pub fn literal(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() + 2);
    out.push('(');
    for b in bytes {
        match b {
            b'(' => out.push_str("\\("),
            b')' => out.push_str("\\)"),
            b'\\' => out.push_str("\\\\"),
            b'\r' => out.push_str("\\r"),
            b'\n' => out.push_str("\\n"),
            0x20..=0x7E => out.push(char::from(*b)),
            other => out.push_str(&format!("\\{other:03o}")),
        }
    }
    out.push(')');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn geometry_matches_hayro() {
        for bytes in [fixtures::text_pages(1), fixtures::rotated_page()] {
            let lo = Document::load_mem(&bytes).expect("lopdf");
            let ids = page_ids(&lo);
            let g = geometry(&lo, ids[0]).expect("geometry");
            let hy = crate::Document::open(bytes.into(), None).expect("hayro");
            let t = hy.user_to_page(0).expect("transform");
            assert!(
                g.to_page
                    .as_coeffs()
                    .iter()
                    .zip(t.as_coeffs())
                    .all(|(a, b)| (a - b).abs() < 1e-6),
                "{:?} vs {:?}",
                g.to_page,
                t
            );
            assert!((g.width as f32 - hy.info().pages[0].width).abs() < 0.01);
        }
    }

    #[test]
    fn numbers_and_strings() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(-0.0001), "0");
        assert_eq!(fmt_num(2.5), "2.5");
        assert_eq!(literal(b"a(b)\\"), "(a\\(b\\)\\\\)");
        assert_eq!(literal(&[0xE9]), "(\\351)");
    }
}
