//! Saving (design D11): `original bytes + one incremental update` with the changed field
//! values and their regenerated appearances, placed visual signatures, and the XFA entry
//! of hybrid forms removed. Earlier revisions (and their signatures) are never touched.
//! Encrypted documents get their new objects encrypted with the document's own key.

use hayro::kurbo::Affine;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

use crate::forms::{FieldKind, FieldValue, FormDoc, text_object};
use crate::lo::{self, fmt_num};
use crate::visual::{Placement, check_rect};
use crate::{PdfError, guard};

/// Trailer keys that belong to the previous section, not the new one.
const STALE_TRAILER_KEYS: [&[u8]; 8] = [
    b"XRefStm",
    b"Type",
    b"W",
    b"Index",
    b"Length",
    b"Filter",
    b"DecodeParms",
    b"Encrypt",
];

impl FormDoc {
    /// Whether anything would be written.
    pub fn has_changes(&self, placements: &[Placement]) -> bool {
        self.is_dirty() || !placements.is_empty()
    }

    /// The original bytes followed by an incremental update with every change. With no
    /// changes this is the original, unchanged.
    pub fn build(&mut self, placements: &[Placement]) -> Result<Vec<u8>, PdfError> {
        if !self.has_changes(placements) {
            return Ok(self.original.to_vec());
        }
        guard(|| self.build_inner(placements))
    }

    fn build_inner(&mut self, placements: &[Placement]) -> Result<Vec<u8>, PdfError> {
        let mut newdoc = Document::new_from_prev(self.inc.get_prev_documents());
        for key in STALE_TRAILER_KEYS {
            newdoc.trailer.remove(key);
        }
        {
            let prev = self.inc.get_prev_documents();
            let mut w = Writer {
                prev,
                new: &mut newdoc,
                form: self,
                wrapped: std::collections::HashSet::new(),
            };
            w.fields()?;
            for (i, p) in placements.iter().enumerate() {
                w.placement(p, i)?;
            }
            if self.xfa {
                w.drop_xfa();
            }
        }
        self.inc.new_document = newdoc;
        let mut out = Vec::with_capacity(self.original.len() + 64 * 1024);
        self.inc
            .save_to(&mut out)
            .map_err(|e| PdfError::Unsupported(format!("writing the update failed ({e})")))?;
        Ok(out)
    }
}

struct Writer<'a> {
    prev: &'a Document,
    new: &'a mut Document,
    form: &'a FormDoc,
    /// Pages whose original content is already wrapped in `q … Q`.
    wrapped: std::collections::HashSet<ObjectId>,
}

impl Writer<'_> {
    /// A copy of object `id` in the update, for changing.
    fn edit(&mut self, id: ObjectId) -> Result<&mut Dictionary, PdfError> {
        if !self.new.has_object(id) {
            let obj = self
                .prev
                .get_object(id)
                .map_err(|_| PdfError::Invalid("an object is missing".into()))?
                .clone();
            self.new.set_object(id, obj);
        }
        match self.new.get_object_mut(id) {
            Ok(Object::Dictionary(d)) => Ok(d),
            Ok(Object::Stream(s)) => Ok(&mut s.dict),
            _ => Err(PdfError::Invalid("an object is not a dictionary".into())),
        }
    }

    fn add(&mut self, obj: impl Into<Object>) -> ObjectId {
        self.new.add_object(obj)
    }

    fn fields(&mut self) -> Result<(), PdfError> {
        let changed: Vec<(String, FieldValue)> = self
            .form
            .changed()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        for (name, value) in changed {
            let Some(field) = self.form.field(&name).cloned() else {
                continue;
            };
            match (&field.kind, &value) {
                (FieldKind::Text { .. }, FieldValue::Text(text)) => {
                    self.edit(field.id)?.set("V", text_object(text));
                    for w in &field.widgets {
                        self.text_widget(&field, w.id, w.page, text, None)?;
                    }
                }
                (FieldKind::Checkbox { .. } | FieldKind::Radio { .. }, FieldValue::State(s)) => {
                    let v = s.clone().unwrap_or_else(|| "Off".into());
                    self.edit(field.id)?
                        .set("V", Object::Name(v.clone().into_bytes()));
                    for w in &field.widgets {
                        let on = w.on_state.as_deref() == Some(v.as_str());
                        let state = if on { v.clone() } else { "Off".to_string() };
                        self.edit(w.id)?.set("AS", Object::Name(state.into_bytes()));
                    }
                }
                (FieldKind::Combo { options, .. }, FieldValue::Selected(sel)) => {
                    let v = sel.first().cloned().unwrap_or_default();
                    let label = options
                        .iter()
                        .find(|o| o.export == v)
                        .map_or(v.clone(), |o| o.label.clone());
                    let d = self.edit(field.id)?;
                    d.set("V", text_object(&v));
                    d.remove(b"I");
                    for w in &field.widgets {
                        self.text_widget(&field, w.id, w.page, &label, None)?;
                    }
                }
                (FieldKind::List { options, .. }, FieldValue::Selected(sel)) => {
                    let indices: Vec<Object> = options
                        .iter()
                        .enumerate()
                        .filter(|(_, o)| sel.contains(&o.export))
                        .map(|(i, _)| Object::Integer(i as i64))
                        .collect();
                    let v = match sel.as_slice() {
                        [] => Object::Null,
                        [one] => text_object(one),
                        many => Object::Array(many.iter().map(|s| text_object(s)).collect()),
                    };
                    let d = self.edit(field.id)?;
                    d.set("V", v);
                    d.set("I", Object::Array(indices));
                    for w in &field.widgets {
                        self.text_widget(&field, w.id, w.page, "", Some((options, sel)))?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// A new normal appearance for a text-like widget.
    fn text_widget(
        &mut self,
        field: &crate::forms::Field,
        widget: ObjectId,
        page: usize,
        text: &str,
        list: Option<(&Vec<crate::forms::ChoiceOption>, &Vec<String>)>,
    ) -> Result<(), PdfError> {
        let wd = self
            .prev
            .get_dictionary(widget)
            .map_err(|_| PdfError::Invalid("a field widget is missing".into()))?;
        let rect = lo::get_rect(self.prev, wd, b"Rect")
            .ok_or_else(|| PdfError::Invalid("a field has no rectangle".into()))?;
        let (w, h) = ((rect[2] - rect[0]).max(1.0), (rect[3] - rect[1]).max(1.0));
        let mut content = self.widget_frame(wd, w, h);
        let (body, font) = self.form.text_appearance(
            self.prev,
            field,
            rect,
            text,
            list.map(|(o, s)| (o.as_slice(), s.as_slice())),
        );
        content.extend_from_slice(&body);
        let mut fonts = Dictionary::new();
        if let Some(name) = font {
            let font_obj = match self.font_ref(&name) {
                Some(r) => r,
                None => Object::Reference(self.add(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1",
                    "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
                })),
            };
            fonts.set(name.into_bytes(), font_obj);
        }
        let _ = page;
        let ap = self.add(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), Object::Real(w as f32), Object::Real(h as f32)],
                "Resources" => dictionary! { "Font" => fonts },
            },
            content,
        ));
        let d = self.edit(widget)?;
        d.set("AP", dictionary! { "N" => ap });
        Ok(())
    }

    /// Background and border from the widget's `/MK`, like other viewers draw them.
    fn widget_frame(&self, wd: &Dictionary, w: f64, h: f64) -> Vec<u8> {
        let mut out = String::new();
        let Some(mk) = lo::get_dict(self.prev, wd, b"MK") else {
            return Vec::new();
        };
        let color = |key: &[u8]| -> Option<String> {
            let Object::Array(a) = lo::get(self.prev, mk, key)? else {
                return None;
            };
            let n: Vec<String> = a
                .iter()
                .filter_map(|o| lo::deref(self.prev, o).and_then(lo::num))
                .map(fmt_num)
                .collect();
            match n.len() {
                1 => Some(format!("{} g", n[0])),
                3 => Some(format!("{} rg", n.join(" "))),
                4 => Some(format!("{} k", n.join(" "))),
                _ => None,
            }
        };
        if let Some(bg) = color(b"BG") {
            out.push_str(&format!(
                "q {bg} 0 0 {} {} re f Q\n",
                fmt_num(w),
                fmt_num(h)
            ));
        }
        if let Some(bc) = color(b"BC") {
            let stroke = bc
                .replace(" rg", " RG")
                .replace(" g", " G")
                .replace(" k", " K");
            out.push_str(&format!(
                "q {stroke} 1 w 0.5 0.5 {} {} re S Q\n",
                fmt_num(w - 1.0),
                fmt_num(h - 1.0)
            ));
        }
        out.into_bytes()
    }

    fn font_ref(&self, name: &str) -> Option<Object> {
        let acro = self.form.acro_dict(self.prev)?;
        let dr = lo::get_dict(self.prev, acro, b"DR")?;
        let fonts = lo::get_dict(self.prev, dr, b"Font")?;
        fonts.get(name.as_bytes()).ok().cloned()
    }

    fn placement(&mut self, p: &Placement, index: usize) -> Result<(), PdfError> {
        let size = self
            .form
            .page_size(p.page)
            .ok_or_else(|| PdfError::Refused(format!("page {} does not exist", p.page + 1)))?;
        check_rect(p.rect, size)?;
        let g = *self
            .form
            .geoms
            .get(p.page)
            .ok_or_else(|| PdfError::Refused("no such page".into()))?;
        let page_id = *self
            .form
            .pages
            .get(p.page)
            .ok_or_else(|| PdfError::Refused("no such page".into()))?;
        // Widget of a signature field: the signature becomes its appearance.
        if let Some(name) = &p.field {
            let field =
                self.form.field(name).cloned().ok_or_else(|| {
                    PdfError::Refused(format!("there is no field named “{name}”"))
                })?;
            let widget = field
                .widgets
                .first()
                .ok_or_else(|| PdfError::Refused("the signature field is not on a page".into()))?;
            let wd = self
                .prev
                .get_dictionary(widget.id)
                .map_err(|_| PdfError::Invalid("a field widget is missing".into()))?;
            let rect = lo::get_rect(self.prev, wd, b"Rect")
                .ok_or_else(|| PdfError::Invalid("a field has no rectangle".into()))?;
            let (w, h) = ((rect[2] - rect[0]).max(1.0), (rect[3] - rect[1]).max(1.0));
            let xobj = self.signature_xobject(p, w, h);
            self.edit(widget.id)?.set("AP", dictionary! { "N" => xobj });
            return Ok(());
        }
        let w = f64::from(p.rect[2] - p.rect[0]);
        let h = f64::from(p.rect[3] - p.rect[1]);
        let xobj = self.signature_xobject(p, w, h);
        let name = format!("FmSig{index}");
        // Upright as displayed: xobject (u, v) → page (x0 + u, y1 − v) → user space.
        let m = g.to_user()
            * Affine::new([
                1.0,
                0.0,
                0.0,
                -1.0,
                f64::from(p.rect[0]),
                f64::from(p.rect[3]),
            ]);
        let c = m.as_coeffs();
        let draw = format!(
            "q {} {} {} {} {} {} cm /{name} Do Q\n",
            fmt_num(c[0]),
            fmt_num(c[1]),
            fmt_num(c[2]),
            fmt_num(c[3]),
            fmt_num(c[4]),
            fmt_num(c[5])
        );
        self.add_to_page(page_id, &name, xobj, draw.into_bytes())
    }

    fn signature_xobject(&mut self, p: &Placement, w: f64, h: f64) -> ObjectId {
        let mut resources = Dictionary::new();
        if let Some(img) = &p.visual.image {
            let mut d = dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => i64::from(img.width), "Height" => i64::from(img.height),
                "ColorSpace" => img.color_space, "BitsPerComponent" => 8,
                "Filter" => img.filter,
            };
            if let Some(mask) = &img.smask {
                let smask = self.add(
                    Stream::new(
                        dictionary! {
                            "Type" => "XObject", "Subtype" => "Image",
                            "Width" => i64::from(img.width), "Height" => i64::from(img.height),
                            "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                            "Filter" => "FlateDecode",
                        },
                        mask.clone(),
                    )
                    .with_compression(false),
                );
                d.set("SMask", smask);
            }
            let im = self.add(Stream::new(d, img.data.clone()).with_compression(false));
            resources.set("XObject", dictionary! { "Im0" => im });
        }
        self.add(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), Object::Real(w as f32), Object::Real(h as f32)],
                "Resources" => resources,
            },
            p.visual.fitted(w, h),
        ))
    }

    /// Draw `xobj` as `/name` on a page: the original content is wrapped in `q … Q` so its
    /// graphics state cannot leak into ours.
    fn add_to_page(
        &mut self,
        page_id: ObjectId,
        name: &str,
        xobj: ObjectId,
        draw: Vec<u8>,
    ) -> Result<(), PdfError> {
        let page = self
            .prev
            .get_dictionary(page_id)
            .map_err(|_| PdfError::Invalid("a page is missing".into()))?;
        // Resources: a direct copy with the XObject added (inherited ones included).
        let mut res = lo::inherited(self.prev, page, b"Resources")
            .and_then(|o| match o {
                Object::Dictionary(d) => Some(d.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let mut xobjects = res
            .get(b"XObject")
            .ok()
            .and_then(|o| lo::dict(self.prev, o))
            .cloned()
            .unwrap_or_default();
        xobjects.set(name.as_bytes().to_vec(), xobj);
        res.set("XObject", xobjects);
        let existing: Vec<Object> = match page.get(b"Contents") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match self.prev.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![Object::Reference(*r)],
            },
            _ => Vec::new(),
        };
        let already_wrapped = !self.wrapped.insert(page_id);
        let ours = self.add(Stream::new(dictionary! {}, draw));
        let page_new = self.edit(page_id)?;
        let mut contents = match page_new.get(b"Contents") {
            Ok(Object::Array(a)) if already_wrapped => a.clone(),
            _ => existing,
        };
        page_new.set("Resources", res);
        if !already_wrapped {
            let q = self.add(Stream::new(dictionary! {}, b"q\n".to_vec()));
            let close = self.add(Stream::new(dictionary! {}, b"\nQ\n".to_vec()));
            contents.insert(0, Object::Reference(q));
            contents.push(Object::Reference(close));
        }
        contents.push(Object::Reference(ours));
        let page_new = self.edit(page_id)?;
        page_new.set("Contents", contents);
        Ok(())
    }

    fn drop_xfa(&mut self) {
        let Some(acro) = self.form.acroform else {
            return;
        };
        match acro {
            crate::forms::AcroRef::Object(id) => {
                if let Ok(d) = self.edit(id) {
                    d.remove(b"XFA");
                }
            }
            crate::forms::AcroRef::Inline(root) => {
                if let Ok(cat) = self.edit(root)
                    && let Ok(Object::Dictionary(a)) = cat.get_mut(b"AcroForm")
                {
                    a.remove(b"XFA");
                }
            }
        }
    }
}

/// Hex string helper for signature placeholders.
pub(crate) fn hex_string(bytes: Vec<u8>) -> Object {
    Object::String(bytes, StringFormat::Hexadecimal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::FieldInput;
    use crate::visual::{Visual, VisualKind};
    use crate::{Document as View, fixtures, render, text};
    use std::sync::Arc;

    fn filled() -> (Arc<[u8]>, FormDoc) {
        let original: Arc<[u8]> = fixtures::form().into();
        let mut f = FormDoc::open(original.clone(), None).expect("form");
        f.set("name", FieldInput::Text("Jane Doe".into()))
            .expect("name");
        f.set("agree", FieldInput::Bool(true)).expect("agree");
        f.set("choice", FieldInput::Text("B".into()))
            .expect("radio");
        f.set("country", FieldInput::Text("Belgium".into()))
            .expect("combo");
        f.set("colors", FieldInput::Many(vec!["Green".into()]))
            .expect("list");
        f.set("address", FieldInput::Text("Main street 1\nTown".into()))
            .expect("address");
        f.set("zip", FieldInput::Text("12345".into())).expect("zip");
        (original, f)
    }

    #[test]
    fn filled_form_round_trips_and_keeps_the_original() {
        let (original, mut f) = filled();
        let out = f.build(&[]).expect("build");
        assert!(
            out.starts_with(&original),
            "an incremental update appends only"
        );
        assert!(out.len() > original.len());
        // Reopens in both parsers with the new values.
        let again = FormDoc::open(out.clone().into(), None).expect("reopen lopdf");
        assert_eq!(
            again.field("name").map(|f| &f.value),
            Some(&FieldValue::Text("Jane Doe".into()))
        );
        assert_eq!(
            again.field("agree").map(|f| &f.value),
            Some(&FieldValue::State(Some("Yes".into())))
        );
        assert_eq!(
            again.field("colors").map(|f| &f.value),
            Some(&FieldValue::Selected(vec!["Green".into()]))
        );
        let view = View::open(out.into(), None).expect("reopen hayro");
        // The appearance is drawn: the text is on the page now.
        let t = text::page_text(&view, 0).expect("text");
        assert!(t.text.contains("Jane Doe"), "{:?}", t.text);
        assert!(t.text.contains("Belgium"), "{:?}", t.text);
        let px =
            render::render_page(&view, 0, 1.0, render::ColorMode::Original, None).expect("render");
        assert!(px.rgba.as_chunks::<4>().0.iter().any(|p| p[0] < 50));
    }

    #[test]
    fn nothing_changed_writes_the_original() {
        let original: Arc<[u8]> = fixtures::form().into();
        let mut f = FormDoc::open(original.clone(), None).expect("form");
        assert_eq!(f.build(&[]).expect("build"), original.to_vec());
    }

    #[test]
    fn xfa_is_dropped_from_hybrid_forms() {
        let mut f = FormDoc::open(fixtures::hybrid_xfa().into(), None).expect("form");
        assert!(f.has_xfa());
        f.set("name", FieldInput::Text("X".into())).expect("set");
        let out = f.build(&[]).expect("build");
        let again = FormDoc::open(out.into(), None).expect("reopen");
        assert!(!again.has_xfa());
    }

    #[test]
    fn visual_signatures_are_placed_on_pages_and_in_fields() {
        let original: Arc<[u8]> = fixtures::text_pages(2).into();
        let mut f = FormDoc::open(original.clone(), None).expect("open");
        let visual = Visual::new(VisualKind::Typed {
            name: "Jane".into(),
        })
        .expect("typed");
        let p = Placement {
            id: 1,
            page: 1,
            rect: [100.0, 700.0, 300.0, 780.0],
            field: None,
            visual: visual.clone(),
        };
        let out = f.build(std::slice::from_ref(&p)).expect("build");
        assert!(out.starts_with(&original));
        let view = View::open(out.into(), None).expect("reopen");
        let px =
            render::render_page(&view, 1, 1.0, render::ColorMode::Original, None).expect("render");
        // Ink inside the placed rectangle, none of it at the top.
        let ink = |x0: u32, y0: u32, x1: u32, y1: u32| {
            (y0..y1)
                .flat_map(|y| (x0..x1).map(move |x| (x, y)))
                .filter(|(x, y)| {
                    let i = ((y * px.width + x) * 4) as usize;
                    u16::from(px.rgba[i + 2]) > u16::from(px.rgba[i]) + 40
                })
                .count()
        };
        assert!(ink(100, 700, 300, 780) > 50, "the signature is drawn there");
        assert_eq!(ink(100, 0, 300, 80), 0);
        // In a signature field.
        let mut f = FormDoc::open(fixtures::form().into(), None).expect("form");
        let p = Placement {
            field: Some("sig".into()),
            page: 0,
            rect: [100.0, 432.0, 300.0, 492.0],
            ..p
        };
        let out = f.build(&[p]).expect("build");
        View::open(out.into(), None).expect("reopen");
        // Out of the page is refused.
        let mut f = FormDoc::open(fixtures::text_pages(1).into(), None).expect("open");
        let bad = Placement {
            id: 2,
            page: 0,
            rect: [500.0, 10.0, 700.0, 50.0],
            field: None,
            visual,
        };
        assert!(f.build(&[bad]).is_err());
    }

    #[test]
    fn encrypted_documents_stay_encrypted() {
        use crate::fixtures::{Cipher, encrypted_with, form_doc};
        use crate::secret::Secret;
        for cipher in [Cipher::Rc4, Cipher::Aes128, Cipher::Aes256] {
            let bytes: Arc<[u8]> =
                encrypted_with(form_doc(), "pw", cipher, lopdf::Permissions::all()).into();
            let pw = Secret::new("pw");
            let mut f = FormDoc::open(bytes.clone(), Some(&pw)).expect("open");
            assert!(f.is_encrypted());
            f.set("name", FieldInput::Text("Secret Agent".into()))
                .expect("set");
            let out = f.build(&[]).expect("build");
            assert!(out.starts_with(&bytes));
            let appended = &out[bytes.len()..];
            assert!(
                !appended.windows(12).any(|w| w == b"Secret Agent"),
                "{cipher:?}: the value is encrypted in the update"
            );
            let out: Arc<[u8]> = out.into();
            assert!(
                View::open(out.clone(), None).is_err(),
                "still needs the password"
            );
            let view = View::open(out.clone(), Some(&pw)).expect("opens with it");
            let t = text::page_text(&view, 0).expect("text");
            assert!(t.text.contains("Secret Agent"), "{cipher:?}: {:?}", t.text);
            let again = FormDoc::open(out, Some(&pw)).expect("lopdf reopen");
            assert_eq!(
                again.field("name").map(|f| &f.value),
                Some(&FieldValue::Text("Secret Agent".into()))
            );
        }
    }

    #[test]
    fn xref_stream_documents() {
        // Rewrite the fixture with an object stream + xref stream, then fill it.
        let mut doc = lopdf::Document::load_mem(&fixtures::form()).expect("load");
        let mut bytes = Vec::new();
        doc.save_modern(&mut bytes).expect("save modern");
        let original: Arc<[u8]> = bytes.into();
        let mut f = FormDoc::open(original.clone(), None).expect("open");
        f.set("name", FieldInput::Text("Modern".into()))
            .expect("set");
        let out = f.build(&[]).expect("build");
        let view = View::open(out.clone().into(), None).expect("reopen hayro");
        assert!(
            text::page_text(&view, 0)
                .expect("text")
                .text
                .contains("Modern")
        );
        FormDoc::open(out.into(), None).expect("reopen lopdf");
    }
}
