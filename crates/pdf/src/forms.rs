//! AcroForm filling (design D6/D7): the field tree with inherited attributes, value
//! validation, the encodability check and appearance streams for changed fields.
//!
//! [`FormDoc`] is the editable document. It is built only when the user starts filling or
//! signing (viewing never parses with lopdf) and holds the decrypted object graph plus the
//! values as edited. [`crate::write`] turns it into `original bytes + incremental update`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use lopdf::{Dictionary, Document, IncrementalDocument, LoadOptions, Object, ObjectId};

use crate::doc::{MAX_FIELDS, MAX_TREE_DEPTH, Permissions};
use crate::lo::{self, Geometry, fmt_num, literal};
use crate::secret::Secret;
use crate::strings::{decode_text, encode_text, winansi_byte};
use crate::{MAX_EDIT_BYTES, PdfError, guard};

/// Longest value accepted for one field (UTF-8 bytes).
pub const MAX_VALUE: usize = 32 * 1024;

/// One choice of a dropdown or list box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    /// What is stored in the document.
    pub export: String,
    /// What the user sees.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    Text {
        multiline: bool,
        comb: bool,
        password: bool,
        max_len: Option<usize>,
    },
    Checkbox {
        /// The "on" appearance state name (usually `Yes`).
        on: String,
    },
    Radio {
        /// One on-state per widget, in widget order.
        states: Vec<String>,
        no_toggle_off: bool,
    },
    Combo {
        options: Vec<ChoiceOption>,
        editable: bool,
    },
    List {
        options: Vec<ChoiceOption>,
        multi: bool,
    },
    PushButton,
    Signature {
        signed: bool,
    },
}

/// A field's value: text, the chosen on-state (`None` is off), or selected options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldValue {
    Text(String),
    State(Option<String>),
    Selected(Vec<String>),
    None,
}

impl FieldValue {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(t) => t.is_empty(),
            Self::State(s) => s.is_none(),
            Self::Selected(v) => v.is_empty(),
            Self::None => true,
        }
    }
}

/// A value as given by a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldInput {
    Text(String),
    Bool(bool),
    Many(Vec<String>),
}

/// Where a field is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    pub page: usize,
    /// Page-space rectangle.
    pub rect: [f32; 4],
    /// Checkbox/radio on-state of this widget.
    pub on_state: Option<String>,
    pub(crate) id: ObjectId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// Fully qualified name (`parent.child`).
    pub name: String,
    pub kind: FieldKind,
    pub read_only: bool,
    pub required: bool,
    /// The value in the document as loaded.
    pub value: FieldValue,
    pub widgets: Vec<Widget>,
    /// Has calculate/format/validate/keystroke scripts (never run).
    pub scripts: bool,
    /// The field's tooltip (`/TU`), shown as its accessible label.
    pub tooltip: Option<String>,
    pub(crate) id: ObjectId,
    pub(crate) da: Option<String>,
    pub(crate) q: i64,
}

impl Field {
    pub fn fillable(&self) -> bool {
        !self.read_only
            && !matches!(
                self.kind,
                FieldKind::PushButton | FieldKind::Signature { .. }
            )
    }
}

/// The editable document.
pub struct FormDoc {
    pub(crate) original: Arc<[u8]>,
    pub(crate) inc: IncrementalDocument,
    pub(crate) pages: Vec<ObjectId>,
    pub(crate) geoms: Vec<Geometry>,
    fields: Vec<Field>,
    /// Edited values by field name (not yet saved).
    values: BTreeMap<String, FieldValue>,
    pub(crate) acroform: Option<AcroRef>,
    pub(crate) xfa: bool,
    pub(crate) encrypted: bool,
    permissions: Option<Permissions>,
}

impl std::fmt::Debug for FormDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FormDoc")
            .field("fields", &self.fields.len())
            .field("changed", &self.values.len())
            .finish_non_exhaustive()
    }
}

/// Where `/AcroForm` lives: its own object, or inline in the catalog.
#[derive(Clone, Copy, Debug)]
pub(crate) enum AcroRef {
    Object(ObjectId),
    Inline(ObjectId),
}

impl FormDoc {
    /// Parse `bytes` for editing (≤ 64 MiB).
    pub fn open(bytes: Arc<[u8]>, password: Option<&Secret>) -> Result<Self, PdfError> {
        if bytes.len() as u64 > MAX_EDIT_BYTES {
            return Err(PdfError::TooLarge(format!(
                "Filling and signing work on PDFs up to {} MiB.",
                MAX_EDIT_BYTES >> 20
            )));
        }
        guard(|| Self::open_inner(bytes, password))
    }

    fn open_inner(bytes: Arc<[u8]>, password: Option<&Secret>) -> Result<Self, PdfError> {
        let pw = password.map(Secret::expose).unwrap_or("");
        let doc = Document::load_mem_with_options(&bytes, LoadOptions::with_password(pw))
            .or_else(|_| Document::load_mem(&bytes))
            .map_err(|_| {
                PdfError::Unsupported("this PDF cannot be opened for filling and signing".into())
            })?;
        if doc.is_encrypted() {
            return Err(PdfError::Password);
        }
        let encrypted = doc.encryption_state.is_some();
        let permissions = doc.encryption_state.as_ref().map(|s| Permissions {
            raw: s.permissions().bits() as i64,
        });
        let pages = lo::page_ids(&doc);
        if pages.is_empty() {
            return Err(PdfError::Invalid("it has no pages".into()));
        }
        let mut geoms = Vec::with_capacity(pages.len());
        for id in &pages {
            geoms.push(lo::geometry(&doc, *id)?);
        }
        let mut form = Self {
            original: bytes.clone(),
            inc: IncrementalDocument::new(),
            pages,
            geoms,
            fields: Vec::new(),
            values: BTreeMap::new(),
            acroform: None,
            xfa: false,
            encrypted,
            permissions,
        };
        form.read_fields(&doc);
        form.inc = IncrementalDocument::create_from(bytes.to_vec(), doc);
        Ok(form)
    }

    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page size in points as displayed.
    pub fn page_size(&self, page: usize) -> Option<(f32, f32)> {
        self.geoms
            .get(page)
            .map(|g| (g.width as f32, g.height as f32))
    }

    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    pub fn has_xfa(&self) -> bool {
        self.xfa
    }

    /// Why filling is not allowed, if it is not.
    pub fn fill_blocked(&self) -> Option<String> {
        if self.permissions.is_some_and(|p| !p.fill_forms()) {
            return Some("The document's permissions do not allow filling in forms.".into());
        }
        None
    }

    /// The current (edited or loaded) value of `name`.
    pub fn value(&self, name: &str) -> Option<&FieldValue> {
        self.values
            .get(name)
            .or_else(|| self.field(name).map(|f| &f.value))
    }

    /// Fields whose value differs from the loaded one.
    pub fn changed(&self) -> &BTreeMap<String, FieldValue> {
        &self.values
    }

    pub fn is_dirty(&self) -> bool {
        !self.values.is_empty()
    }

    /// Required fields that are still empty.
    pub fn empty_required(&self) -> Vec<String> {
        self.fields
            .iter()
            .filter(|f| f.required && f.fillable())
            .filter(|f| self.value(&f.name).is_none_or(FieldValue::is_empty))
            .map(|f| f.name.clone())
            .collect()
    }

    /// Replace the edited values with `values`, validated elsewhere by a twin
    /// `FormDoc` of the same bytes; names that are not fillable fields are ignored.
    pub fn apply_values(&mut self, values: &BTreeMap<String, FieldValue>) {
        self.values = values
            .iter()
            .filter(|(k, _)| self.field(k).is_some_and(|f| f.fillable() && !f.read_only))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
    }

    /// Forget every edited value.
    pub fn reset(&mut self) {
        self.values.clear();
    }

    /// Set `name` to `input`, after validating it. On error nothing changes.
    pub fn set(&mut self, name: &str, input: FieldInput) -> Result<(), PdfError> {
        if let Some(why) = self.fill_blocked() {
            return Err(PdfError::Refused(why));
        }
        let field = self
            .field(name)
            .ok_or_else(|| PdfError::Refused(format!("there is no field named “{name}”")))?
            .clone();
        if field.read_only {
            return Err(PdfError::Refused(format!("“{name}” is read-only")));
        }
        let value = self.validate(&field, input)?;
        if value == field.value {
            self.values.remove(name);
        } else {
            self.values.insert(field.name.clone(), value);
        }
        Ok(())
    }

    fn validate(&self, field: &Field, input: FieldInput) -> Result<FieldValue, PdfError> {
        let too_long = |s: &str| s.len() > MAX_VALUE;
        match (&field.kind, input) {
            (
                FieldKind::Text {
                    max_len, multiline, ..
                },
                FieldInput::Text(text),
            ) => {
                if too_long(&text) {
                    return Err(PdfError::Refused("the value is longer than 32 KiB".into()));
                }
                let text = if *multiline {
                    text.replace("\r\n", "\n")
                } else {
                    text.replace(['\r', '\n'], " ")
                };
                if let Some(max) = max_len
                    && text.chars().count() > *max
                {
                    return Err(PdfError::Refused(format!(
                        "“{}” takes at most {max} characters",
                        field.name
                    )));
                }
                self.check_encodable(field, &text)?;
                Ok(FieldValue::Text(text))
            }
            (FieldKind::Checkbox { on }, FieldInput::Bool(b)) => {
                Ok(FieldValue::State(b.then(|| on.clone())))
            }
            (FieldKind::Checkbox { on }, FieldInput::Text(t)) => {
                if t == *on {
                    Ok(FieldValue::State(Some(on.clone())))
                } else if t == "Off" || t.is_empty() {
                    Ok(FieldValue::State(None))
                } else {
                    Err(PdfError::Refused(format!(
                        "“{t}” is not a state of this checkbox"
                    )))
                }
            }
            (
                FieldKind::Radio {
                    states,
                    no_toggle_off,
                },
                FieldInput::Text(t),
            ) => {
                if t == "Off" || t.is_empty() {
                    if *no_toggle_off && !field.value.is_empty() {
                        return Err(PdfError::Refused(
                            "one of these options must stay selected".into(),
                        ));
                    }
                    return Ok(FieldValue::State(None));
                }
                if states.contains(&t) {
                    Ok(FieldValue::State(Some(t)))
                } else {
                    Err(PdfError::Refused(format!(
                        "“{t}” is not one of the options"
                    )))
                }
            }
            (FieldKind::Combo { options, editable }, FieldInput::Text(t)) => {
                if too_long(&t) {
                    return Err(PdfError::Refused("the value is longer than 32 KiB".into()));
                }
                if t.is_empty() {
                    return Ok(FieldValue::Selected(Vec::new()));
                }
                let known = options.iter().find(|o| o.export == t || o.label == t);
                match (known, editable) {
                    (Some(o), _) => {
                        self.check_encodable(field, &o.label)?;
                        Ok(FieldValue::Selected(vec![o.export.clone()]))
                    }
                    (None, true) => {
                        self.check_encodable(field, &t)?;
                        Ok(FieldValue::Selected(vec![t]))
                    }
                    (None, false) => Err(PdfError::Refused(format!(
                        "“{t}” is not one of the options"
                    ))),
                }
            }
            (FieldKind::List { options, multi }, input) => {
                let picked = match input {
                    FieldInput::Text(t) if t.is_empty() => Vec::new(),
                    FieldInput::Text(t) => vec![t],
                    FieldInput::Many(v) => v,
                    FieldInput::Bool(_) => {
                        return Err(PdfError::Refused("a list box takes option names".into()));
                    }
                };
                if picked.len() > 1 && !multi {
                    return Err(PdfError::Refused("only one option can be selected".into()));
                }
                let mut out = Vec::new();
                for p in picked {
                    let o = options
                        .iter()
                        .find(|o| o.export == p || o.label == p)
                        .ok_or_else(|| {
                            PdfError::Refused(format!("“{p}” is not one of the options"))
                        })?;
                    if !out.contains(&o.export) {
                        out.push(o.export.clone());
                    }
                }
                Ok(FieldValue::Selected(out))
            }
            (FieldKind::PushButton, _) => Err(PdfError::Refused("buttons have no value".into())),
            (FieldKind::Signature { .. }, _) => Err(PdfError::Refused(
                "signature fields are signed, not filled".into(),
            )),
            _ => Err(PdfError::Refused(format!(
                "that kind of value does not fit “{}”",
                field.name
            ))),
        }
    }

    /// Refuse characters the field's font cannot show (D7).
    fn check_encodable(&self, field: &Field, text: &str) -> Result<(), PdfError> {
        let doc = self.inc.get_prev_documents();
        let font = self.field_font(doc, field);
        for ch in text.chars() {
            if ch == '\n' {
                continue;
            }
            let ok = match &font {
                FontEnc::Simple { .. } => winansi_byte(ch).is_some(),
                FontEnc::Cid { map } => map.contains_key(&ch),
                FontEnc::None => winansi_byte(ch).is_some(),
            };
            if !ok {
                return Err(PdfError::Refused(format!(
                    "The character “{ch}” (U+{:04X}) cannot be shown in this field's font",
                    ch as u32
                )));
            }
        }
        Ok(())
    }

    fn read_fields(&mut self, doc: &Document) {
        let Ok(catalog) = doc.catalog() else {
            return;
        };
        let (acro, acro_ref) = match catalog.get(b"AcroForm") {
            Ok(Object::Reference(id)) => (doc.get_dictionary(*id).ok(), Some(AcroRef::Object(*id))),
            Ok(Object::Dictionary(d)) => {
                let root = doc
                    .trailer
                    .get(b"Root")
                    .ok()
                    .and_then(|r| r.as_reference().ok());
                (Some(d), root.map(AcroRef::Inline))
            }
            _ => (None, None),
        };
        let Some(acro) = acro else {
            return;
        };
        self.acroform = acro_ref;
        self.xfa = acro.has(b"XFA");
        // Widget → page, from every page's /Annots.
        let mut widget_page: HashMap<ObjectId, usize> = HashMap::new();
        for (ix, pid) in self.pages.iter().enumerate() {
            if let Ok(page) = doc.get_dictionary(*pid)
                && let Some(Object::Array(annots)) = lo::get(doc, page, b"Annots")
            {
                for a in annots.iter().take(10_000) {
                    if let Object::Reference(id) = a {
                        widget_page.insert(*id, ix);
                    }
                }
            }
        }
        let mut seen = HashSet::new();
        let mut count = 0usize;
        if let Some(Object::Array(fields)) = lo::get(doc, acro, b"Fields") {
            for f in fields.iter().take(MAX_FIELDS) {
                if let Object::Reference(id) = f {
                    self.walk(
                        doc,
                        *id,
                        &Inherit::default(),
                        0,
                        &mut seen,
                        &mut count,
                        &widget_page,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        doc: &Document,
        id: ObjectId,
        parent: &Inherit,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        count: &mut usize,
        widget_page: &HashMap<ObjectId, usize>,
    ) {
        *count += 1;
        if depth > MAX_TREE_DEPTH || *count > MAX_FIELDS || !seen.insert(id) {
            return;
        }
        let Ok(d) = doc.get_dictionary(id) else {
            return;
        };
        let mut inh = parent.clone();
        if let Some(t) = lo::get_text(doc, d, b"T") {
            inh.name = if inh.name.is_empty() {
                t
            } else {
                format!("{}.{t}", inh.name)
            };
        }
        if let Some(ft) = lo::get_name(doc, d, b"FT") {
            inh.ft = Some(ft);
        }
        if let Some(ff) = lo::get_num(doc, d, b"Ff") {
            inh.ff = ff as i64;
        }
        if let Some(da) = lo::get_text(doc, d, b"DA") {
            inh.da = Some(da);
        }
        if let Some(q) = lo::get_num(doc, d, b"Q") {
            inh.q = q as i64;
        }
        if let Some(m) = lo::get_num(doc, d, b"MaxLen") {
            inh.max_len = Some(m.clamp(0.0, 100_000.0) as usize);
        }
        if d.has(b"V") {
            inh.v = d.get(b"V").ok().cloned();
        }
        if d.has(b"Opt") {
            inh.opt = d.get(b"Opt").ok().cloned();
        }
        if d.has(b"AA") {
            inh.scripts = true;
        }
        if let Some(tu) = lo::get_text(doc, d, b"TU") {
            inh.tooltip = Some(tu);
        }
        let kids: Vec<ObjectId> = match lo::get(doc, d, b"Kids") {
            Some(Object::Array(k)) => k
                .iter()
                .filter_map(|o| o.as_reference().ok())
                .take(MAX_FIELDS)
                .collect(),
            _ => Vec::new(),
        };
        let field_kids: Vec<ObjectId> = kids
            .iter()
            .copied()
            .filter(|k| doc.get_dictionary(*k).is_ok_and(|kd| kd.has(b"T")))
            .collect();
        if !field_kids.is_empty() {
            for k in field_kids {
                self.walk(doc, k, &inh, depth + 1, seen, count, widget_page);
            }
            return;
        }
        // Terminal field: its widgets are its kids, or itself.
        let widget_ids = if kids.is_empty() { vec![id] } else { kids };
        let mut widgets = Vec::new();
        for wid in widget_ids {
            let Ok(w) = doc.get_dictionary(wid) else {
                continue;
            };
            if w.has(b"AA") {
                inh.scripts = true;
            }
            let page = widget_page.get(&wid).copied().or_else(|| {
                let p = w.get(b"P").ok()?.as_reference().ok()?;
                self.pages.iter().position(|x| *x == p)
            });
            let (Some(page), Some(rect)) = (page, lo::get_rect(doc, w, b"Rect")) else {
                continue;
            };
            let Some(g) = self.geoms.get(page) else {
                continue;
            };
            let on_state = lo::get_dict(doc, w, b"AP")
                .and_then(|ap| lo::get_dict(doc, ap, b"N"))
                .and_then(|n| {
                    n.iter()
                        .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                        .find(|k| k != "Off")
                });
            widgets.push(Widget {
                page,
                rect: g.rect_to_page(rect),
                on_state,
                id: wid,
            });
        }
        let ff = inh.ff;
        let bit = |n: u32| ff & (1 << (n - 1)) != 0;
        let options = || parse_options(doc, inh.opt.as_ref());
        let kind = match inh.ft.as_deref() {
            Some("Tx") => FieldKind::Text {
                multiline: bit(13),
                password: bit(14),
                comb: bit(25) && inh.max_len.is_some() && !bit(13) && !bit(14),
                max_len: inh.max_len,
            },
            Some("Btn") if bit(17) => FieldKind::PushButton,
            Some("Btn") if bit(16) => FieldKind::Radio {
                states: widgets.iter().filter_map(|w| w.on_state.clone()).collect(),
                no_toggle_off: bit(15),
            },
            Some("Btn") => FieldKind::Checkbox {
                on: widgets
                    .iter()
                    .find_map(|w| w.on_state.clone())
                    .unwrap_or_else(|| "Yes".into()),
            },
            Some("Ch") if bit(18) => FieldKind::Combo {
                options: options(),
                editable: bit(19),
            },
            Some("Ch") => FieldKind::List {
                options: options(),
                multi: bit(22),
            },
            Some("Sig") => FieldKind::Signature {
                signed: inh.v.as_ref().is_some_and(|v| !matches!(v, Object::Null)),
            },
            _ => return,
        };
        let value = match (&kind, inh.v.as_ref().and_then(|v| lo::deref(doc, v))) {
            (FieldKind::Text { .. }, Some(Object::String(s, _))) => {
                FieldValue::Text(decode_text(s))
            }
            (FieldKind::Text { .. }, Some(Object::Stream(s))) => FieldValue::Text(decode_text(
                &s.decompressed_content_with_limit(MAX_VALUE)
                    .unwrap_or_default(),
            )),
            (FieldKind::Text { .. }, _) => FieldValue::Text(String::new()),
            (FieldKind::Checkbox { .. } | FieldKind::Radio { .. }, Some(Object::Name(n))) => {
                let n = String::from_utf8_lossy(n).into_owned();
                FieldValue::State((n != "Off").then_some(n))
            }
            (FieldKind::Checkbox { .. } | FieldKind::Radio { .. }, _) => FieldValue::State(None),
            (FieldKind::Combo { .. } | FieldKind::List { .. }, Some(Object::String(s, _))) => {
                FieldValue::Selected(vec![decode_text(s)])
            }
            (FieldKind::Combo { .. } | FieldKind::List { .. }, Some(Object::Array(a))) => {
                FieldValue::Selected(
                    a.iter()
                        .filter_map(|o| match lo::deref(doc, o) {
                            Some(Object::String(s, _)) => Some(decode_text(s)),
                            _ => None,
                        })
                        .take(1000)
                        .collect(),
                )
            }
            (FieldKind::Combo { .. } | FieldKind::List { .. }, _) => {
                FieldValue::Selected(Vec::new())
            }
            _ => FieldValue::None,
        };
        self.fields.push(Field {
            name: inh.name.clone(),
            kind,
            read_only: bit(1),
            required: bit(2),
            value,
            widgets,
            scripts: inh.scripts,
            tooltip: inh.tooltip.clone(),
            id,
            da: inh.da.clone().or_else(|| self.acro_da(doc)),
            q: inh.q,
        });
    }

    fn acro_da(&self, doc: &Document) -> Option<String> {
        let acro = self.acro_dict(doc)?;
        lo::get_text(doc, acro, b"DA")
    }

    pub(crate) fn acro_dict<'a>(&self, doc: &'a Document) -> Option<&'a Dictionary> {
        match self.acroform? {
            AcroRef::Object(id) => doc.get_dictionary(id).ok(),
            AcroRef::Inline(root) => {
                let cat = doc.get_dictionary(root).ok()?;
                lo::get_dict(doc, cat, b"AcroForm")
            }
        }
    }

    /// The `/DR` font named by the field's `/DA`.
    fn field_font(&self, doc: &Document, field: &Field) -> FontEnc {
        let Some(da) = field.da.as_deref() else {
            return FontEnc::None;
        };
        let Some(name) = parse_da(da).font else {
            return FontEnc::None;
        };
        let Some(font) = self.dr_font(doc, &name) else {
            return FontEnc::None;
        };
        font_encoding(doc, font)
    }

    pub(crate) fn dr_font<'a>(&self, doc: &'a Document, name: &str) -> Option<&'a Dictionary> {
        let acro = self.acro_dict(doc)?;
        let dr = lo::get_dict(doc, acro, b"DR")?;
        let fonts = lo::get_dict(doc, dr, b"Font")?;
        lo::get_dict(doc, fonts, name.as_bytes())
    }
}

#[derive(Clone, Debug, Default)]
struct Inherit {
    name: String,
    ft: Option<String>,
    ff: i64,
    da: Option<String>,
    q: i64,
    max_len: Option<usize>,
    v: Option<Object>,
    opt: Option<Object>,
    scripts: bool,
    tooltip: Option<String>,
}

fn parse_options(doc: &Document, opt: Option<&Object>) -> Vec<ChoiceOption> {
    let Some(Object::Array(a)) = opt.and_then(|o| lo::deref(doc, o)) else {
        return Vec::new();
    };
    a.iter()
        .take(10_000)
        .filter_map(|o| match lo::deref(doc, o)? {
            Object::String(s, _) => {
                let t = decode_text(s);
                Some(ChoiceOption {
                    export: t.clone(),
                    label: t,
                })
            }
            Object::Array(pair) if pair.len() == 2 => {
                let text = |o: &Object| match lo::deref(doc, o) {
                    Some(Object::String(s, _)) => Some(decode_text(s)),
                    _ => None,
                };
                Some(ChoiceOption {
                    export: text(&pair[0])?,
                    label: text(&pair[1])?,
                })
            }
            _ => None,
        })
        .collect()
}

/// The parts of a `/DA` string appearance generation needs.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Da {
    pub font: Option<String>,
    pub size: f64,
    /// Colour operator with operands (`0 g`, `1 0 0 rg`, …).
    pub color: String,
}

pub(crate) fn parse_da(da: &str) -> Da {
    let tokens: Vec<&str> = da.split_whitespace().take(64).collect();
    let mut out = Da {
        color: "0 g".into(),
        size: 0.0,
        ..Default::default()
    };
    for (i, t) in tokens.iter().enumerate() {
        match *t {
            "Tf" if i >= 2 => {
                out.font = tokens
                    .get(i - 2)
                    .and_then(|n| n.strip_prefix('/'))
                    .map(str::to_string);
                out.size = tokens
                    .get(i - 1)
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|s| s.is_finite() && *s >= 0.0 && *s <= 300.0)
                    .unwrap_or(0.0);
            }
            "g" | "rg" | "k" => {
                let n = match *t {
                    "g" => 1,
                    "rg" => 3,
                    _ => 4,
                };
                if i >= n {
                    let nums: Vec<&str> = tokens[i - n..i].to_vec();
                    if nums.iter().all(|x| x.parse::<f64>().is_ok()) {
                        out.color = format!("{} {t}", nums.join(" "));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// How text is encoded for a field font.
pub(crate) enum FontEnc {
    /// A simple font: WinAnsi bytes, glyph widths (1000/em) by code.
    Simple {
        widths: Box<[f64; 256]>,
    },
    /// A composite font: Unicode → 2-byte code from its ToUnicode map.
    Cid {
        map: HashMap<char, u16>,
    },
    None,
}

pub(crate) fn font_encoding(doc: &Document, font: &Dictionary) -> FontEnc {
    let subtype = lo::get_name(doc, font, b"Subtype").unwrap_or_default();
    if subtype == "Type0" {
        let map = lo::get(doc, font, b"ToUnicode")
            .and_then(|o| match o {
                Object::Stream(s) => s.decompressed_content_with_limit(1 << 20).ok(),
                _ => None,
            })
            .map(|data| parse_to_unicode(&data))
            .unwrap_or_default();
        return FontEnc::Cid { map };
    }
    let base = lo::get_name(doc, font, b"BaseFont").unwrap_or_default();
    let mut widths = Box::new([0.0f64; 256]);
    for (code, w) in widths.iter_mut().enumerate() {
        *w = standard_width(&base, code as u8);
    }
    if let (Some(first), Some(Object::Array(ws))) = (
        lo::get_num(doc, font, b"FirstChar"),
        lo::get(doc, font, b"Widths"),
    ) {
        let first = first.clamp(0.0, 255.0) as usize;
        for (i, w) in ws.iter().enumerate() {
            if let (Some(slot), Some(v)) = (
                widths.get_mut(first + i),
                lo::deref(doc, w).and_then(lo::num),
            ) && v.is_finite()
                && v > 0.0
            {
                *slot = v;
            }
        }
    }
    FontEnc::Simple { widths }
}

/// Reverse a ToUnicode CMap: Unicode char → 2-byte code (single chars only).
fn parse_to_unicode(data: &[u8]) -> HashMap<char, u16> {
    let text = String::from_utf8_lossy(data);
    let mut map = HashMap::new();
    let hex = |s: &str| -> Option<Vec<u8>> {
        let s = s.trim().trim_start_matches('<').trim_end_matches('>');
        if !s.len().is_multiple_of(2) || s.len() > 64 {
            return None;
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect()
    };
    let code_of = |b: &[u8]| -> Option<u16> {
        match b {
            [a] => Some(u16::from(*a)),
            [a, b] => Some(u16::from_be_bytes([*a, *b])),
            _ => None,
        }
    };
    let char_of = |b: &[u8]| -> Option<char> {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        let mut it = char::decode_utf16(units);
        let c = it.next()?.ok()?;
        it.next().is_none().then_some(c)
    };
    let add = |c: char, code: u16, map: &mut HashMap<char, u16>| {
        if map.len() < 65_536 {
            map.entry(c).or_insert(code);
        }
    };
    for section in text.split("beginbfchar").skip(1) {
        let body = section.split("endbfchar").next().unwrap_or("");
        let tokens: Vec<&str> = body.split_whitespace().collect();
        for pair in tokens.chunks(2) {
            if let [src, dst] = pair
                && let (Some(src), Some(dst)) = (hex(src), hex(dst))
                && let (Some(code), Some(c)) = (code_of(&src), char_of(&dst))
            {
                add(c, code, &mut map);
            }
        }
    }
    for section in text.split("beginbfrange").skip(1) {
        let body = section.split("endbfrange").next().unwrap_or("");
        // `<lo> <hi> <dst>` or `<lo> <hi> [<d0> <d1> …]`
        let mut rest = body;
        for _ in 0..10_000 {
            let mut toks = rest.trim_start().splitn(3, char::is_whitespace);
            let (Some(lo_t), Some(hi_t), Some(tail)) = (toks.next(), toks.next(), toks.next())
            else {
                break;
            };
            let (Some(lo_b), Some(hi_b)) = (hex(lo_t), hex(hi_t)) else {
                break;
            };
            let (Some(lo_c), Some(hi_c)) = (code_of(&lo_b), code_of(&hi_b)) else {
                break;
            };
            let tail = tail.trim_start();
            if let Some(arr) = tail.strip_prefix('[') {
                let end = arr.find(']').unwrap_or(arr.len());
                for (i, d) in arr[..end].split_whitespace().enumerate() {
                    if let (Some(d), Some(code)) = (hex(d), lo_c.checked_add(i as u16))
                        && let Some(c) = char_of(&d)
                        && code <= hi_c
                    {
                        add(c, code, &mut map);
                    }
                }
                rest = arr.get(end + 1..).unwrap_or("");
            } else {
                let end = tail.find('>').map_or(tail.len(), |e| e + 1);
                if let Some(d) = hex(&tail[..end])
                    && let Some(c) = char_of(&d)
                {
                    let span = hi_c.saturating_sub(lo_c).min(4096);
                    for i in 0..=span {
                        if let Some(ch) = char::from_u32(c as u32 + u32::from(i)) {
                            add(ch, lo_c + i, &mut map);
                        }
                    }
                }
                rest = tail.get(end..).unwrap_or("");
            }
        }
    }
    map
}

/// Standard 14 widths (1000/em) for WinAnsi codes; Courier is monospaced.
fn standard_width(base: &str, code: u8) -> f64 {
    const HELV: [u16; 95] = [
        278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556,
        556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722,
        722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722,
        667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
        556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500,
        500, 334, 260, 334, 584,
    ];
    const TIMES: [u16; 95] = [
        250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500,
        500, 500, 500, 500, 500, 500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667,
        722, 611, 556, 722, 722, 333, 389, 722, 611, 889, 722, 722, 556, 722, 667, 556, 611, 722,
        722, 944, 722, 722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500, 444, 333, 500,
        500, 278, 278, 500, 278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500,
        444, 480, 200, 480, 541,
    ];
    let lower = base.to_ascii_lowercase();
    if lower.contains("courier") {
        return 600.0;
    }
    let table = if lower.contains("times") || lower.contains("serif") && !lower.contains("sans") {
        &TIMES
    } else {
        &HELV
    };
    match code {
        32..=126 => f64::from(table[(code - 32) as usize]),
        // Latin-1 letters: about the width of their base letters.
        0xC0..=0xDE => f64::from(table[(b'A' - 32) as usize]),
        0xDF..=0xFF => f64::from(table[(b'a' - 32) as usize]),
        _ => f64::from(table[(b'n' - 32) as usize]),
    }
}

/// Text laid out for an appearance stream: font resource name, size, and encoded lines.
pub(crate) struct Layout {
    pub lines: Vec<(f64, f64, Vec<u8>, bool)>,
    pub size: f64,
}

impl FormDoc {
    /// Build the normal-appearance content for a text-like widget.
    pub(crate) fn text_appearance(
        &self,
        doc: &Document,
        field: &Field,
        widget_rect_user: [f64; 4],
        text: &str,
        list: Option<(&[ChoiceOption], &[String])>,
    ) -> (Vec<u8>, Option<String>) {
        let w = (widget_rect_user[2] - widget_rect_user[0]).max(1.0);
        let h = (widget_rect_user[3] - widget_rect_user[1]).max(1.0);
        let da = parse_da(field.da.as_deref().unwrap_or("/Helv 0 Tf 0 g"));
        let font_name = da.font.clone();
        let enc = font_name
            .as_deref()
            .and_then(|n| self.dr_font(doc, n))
            .map(|f| font_encoding(doc, f))
            .unwrap_or(FontEnc::None);
        let measure = |s: &str, size: f64| -> f64 {
            let units: f64 = s
                .chars()
                .map(|c| match &enc {
                    FontEnc::Simple { widths } => winansi_byte(c)
                        .map_or(500.0, |b| widths.get(b as usize).copied().unwrap_or(500.0)),
                    _ => standard_width("Helvetica", winansi_byte(c).unwrap_or(b'n')),
                })
                .sum();
            units * size / 1000.0
        };
        let encode = |s: &str| -> (Vec<u8>, bool) {
            match &enc {
                FontEnc::Cid { map } => (
                    s.chars()
                        .filter_map(|c| map.get(&c))
                        .flat_map(|c| c.to_be_bytes())
                        .collect(),
                    true,
                ),
                _ => (s.chars().filter_map(winansi_byte).collect(), false),
            }
        };
        let pad = 2.0;
        let mut out = String::new();
        out.push_str("/Tx BMC\nq\n");
        out.push_str(&format!(
            "{} {} {} {} re W n\n",
            fmt_num(1.0),
            fmt_num(1.0),
            fmt_num(w - 2.0),
            fmt_num(h - 2.0)
        ));
        let (multiline, comb, password, max_len) = match &field.kind {
            FieldKind::Text {
                multiline,
                comb,
                password,
                max_len,
            } => (*multiline, *comb, *password, *max_len),
            _ => (false, false, false, None),
        };
        let shown: String = if password {
            "*".repeat(text.chars().count())
        } else {
            text.to_string()
        };
        let mut layout = Layout {
            lines: Vec::new(),
            size: da.size,
        };
        if let Some((options, selected)) = list {
            let size = if da.size > 0.0 { da.size } else { 12.0 };
            let line_h = size * 1.15;
            let mut y = h - pad - size;
            for o in options {
                if y < -size {
                    break;
                }
                if selected.contains(&o.export) {
                    out.push_str(&format!(
                        "0.6 0.75 0.85 rg 1 {} {} {} re f\n",
                        fmt_num(y - size * 0.25),
                        fmt_num(w - 2.0),
                        fmt_num(line_h)
                    ));
                }
                let (bytes, cid) = encode(&o.label);
                layout.lines.push((pad, y, bytes, cid));
                y -= line_h;
            }
            layout.size = size;
        } else if multiline {
            let mut size = if da.size > 0.0 { da.size } else { 12.0 };
            loop {
                let lines = wrap(&shown, w - 2.0 * pad, |s| measure(s, size));
                let need = lines.len() as f64 * size * 1.15 + pad;
                if need <= h || size <= 4.0 || da.size > 0.0 {
                    let mut y = h - pad - size;
                    for line in lines {
                        let lw = measure(&line, size);
                        let x = align_x(field.q, w, lw, pad);
                        let (bytes, cid) = encode(&line);
                        layout.lines.push((x, y, bytes, cid));
                        y -= size * 1.15;
                    }
                    layout.size = size;
                    break;
                }
                size -= 0.5;
            }
        } else if comb && max_len.is_some_and(|m| m > 0) {
            let cells = max_len.unwrap_or(1).max(1) as f64;
            let size = if da.size > 0.0 {
                da.size
            } else {
                ((h - 2.0 * pad) * 0.75).clamp(4.0, 12.0)
            };
            let cell = w / cells;
            let y = (h - size) / 2.0 + size * 0.22;
            for (i, c) in shown.chars().enumerate() {
                let s = c.to_string();
                let cw = measure(&s, size);
                let x = cell * i as f64 + (cell - cw) / 2.0;
                let (bytes, cid) = encode(&s);
                layout.lines.push((x, y, bytes, cid));
            }
            layout.size = size;
        } else {
            let mut size = if da.size > 0.0 {
                da.size
            } else {
                ((h - 2.0 * pad) * 0.75).clamp(4.0, 12.0)
            };
            if da.size <= 0.0 {
                let tw = measure(&shown, size);
                if tw > w - 2.0 * pad && tw > 0.0 {
                    size = (size * (w - 2.0 * pad) / tw).max(4.0);
                }
            }
            let tw = measure(&shown, size);
            let x = align_x(field.q, w, tw, pad);
            let y = (h - size) / 2.0 + size * 0.22;
            let (bytes, cid) = encode(&shown);
            layout.lines.push((x, y, bytes, cid));
            layout.size = size;
        }
        if let Some(font) = &font_name {
            out.push_str("BT\n");
            out.push_str(&format!(
                "/{font} {} Tf\n{}\n",
                fmt_num(layout.size),
                da.color
            ));
            for (x, y, bytes, cid) in &layout.lines {
                let s = if *cid {
                    format!(
                        "<{}>",
                        bytes.iter().map(|b| format!("{b:02X}")).collect::<String>()
                    )
                } else {
                    literal(bytes)
                };
                out.push_str(&format!(
                    "1 0 0 1 {} {} Tm {s} Tj\n",
                    fmt_num(*x),
                    fmt_num(*y)
                ));
            }
            out.push_str("ET\n");
        }
        out.push_str("Q\nEMC\n");
        (out.into_bytes(), font_name)
    }
}

fn align_x(q: i64, w: f64, tw: f64, pad: f64) -> f64 {
    match q {
        1 => ((w - tw) / 2.0).max(pad),
        2 => (w - pad - tw).max(pad),
        _ => pad,
    }
}

/// Greedy word wrap within `width`, honouring newlines.
fn wrap(text: &str, width: f64, measure: impl Fn(&str) -> f64) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let candidate = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if measure(&candidate) <= width || line.is_empty() {
                line = candidate;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
            if out.len() > 2000 {
                return out;
            }
        }
        out.push(line);
    }
    out
}

/// Text field values are stored as PDF text strings.
pub(crate) fn text_object(text: &str) -> Object {
    Object::String(encode_text(text), lopdf::StringFormat::Literal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn form() -> FormDoc {
        FormDoc::open(fixtures::form().into(), None).expect("form")
    }

    #[test]
    fn reads_every_field_type() {
        let f = form();
        let names: Vec<&str> = f.fields().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "name", "address", "zip", "agree", "choice", "country", "colors", "sig"
            ]
        );
        let name = f.field("name").expect("name");
        assert!(name.required && !name.read_only);
        assert_eq!(name.widgets.len(), 1);
        let r = name.widgets[0].rect;
        assert!(
            (r[0] - 100.0).abs() < 0.01 && (r[1] - 122.0).abs() < 0.01,
            "{r:?}"
        );
        assert!(matches!(
            f.field("zip").map(|z| &z.kind),
            Some(FieldKind::Text {
                comb: true,
                max_len: Some(5),
                ..
            })
        ));
        assert!(matches!(
            f.field("address").map(|z| &z.kind),
            Some(FieldKind::Text {
                multiline: true,
                ..
            })
        ));
        assert_eq!(
            f.field("agree").map(|a| &a.kind),
            Some(&FieldKind::Checkbox { on: "Yes".into() })
        );
        assert!(matches!(
            f.field("choice").map(|c| &c.kind),
            Some(FieldKind::Radio { states, no_toggle_off: true }) if states == &["A", "B"]
        ));
        assert!(matches!(
            f.field("country").map(|c| &c.kind),
            Some(FieldKind::Combo { options, editable: false }) if options.len() == 3
        ));
        assert!(matches!(
            f.field("colors").map(|c| &c.kind),
            Some(FieldKind::List { multi: true, .. })
        ));
        assert!(matches!(
            f.field("sig").map(|c| &c.kind),
            Some(FieldKind::Signature { signed: false })
        ));
        assert_eq!(f.empty_required(), ["name"]);
    }

    #[test]
    fn set_validates_and_never_half_applies() {
        let mut f = form();
        f.set("name", FieldInput::Text("Jane Doe".into()))
            .expect("text");
        assert_eq!(f.value("name"), Some(&FieldValue::Text("Jane Doe".into())));
        assert!(f.empty_required().is_empty());
        f.set("agree", FieldInput::Bool(true)).expect("check");
        assert_eq!(
            f.value("agree"),
            Some(&FieldValue::State(Some("Yes".into())))
        );
        f.set("choice", FieldInput::Text("B".into()))
            .expect("radio");
        f.set("country", FieldInput::Text("Belgium".into()))
            .expect("combo");
        f.set(
            "colors",
            FieldInput::Many(vec!["Red".into(), "Blue".into()]),
        )
        .expect("multi list");
        assert_eq!(f.changed().len(), 5);
        // Refusals leave everything as it was.
        let before = f.changed().clone();
        for (field, input) in [
            ("nope", FieldInput::Text("x".into())),
            ("country", FieldInput::Text("France".into())),
            ("zip", FieldInput::Text("123456".into())),
            ("choice", FieldInput::Text("C".into())),
            ("name", FieldInput::Text("x".repeat(MAX_VALUE + 1))),
            ("name", FieldInput::Text("日本".into())),
            ("sig", FieldInput::Text("x".into())),
            ("agree", FieldInput::Many(vec![])),
        ] {
            assert!(f.set(field, input).is_err(), "{field} must refuse");
        }
        assert_eq!(f.changed(), &before);
        let err = f
            .set("name", FieldInput::Text("Zoë 日".into()))
            .expect_err("unencodable");
        assert!(err.to_string().contains("日"), "{err}");
        f.reset();
        assert!(!f.is_dirty());
    }

    #[test]
    fn restricted_documents_refuse_filling() {
        let f = FormDoc::open(fixtures::restricted_form().into(), None).expect("open");
        assert!(f.is_encrypted());
        assert!(f.fill_blocked().is_some());
    }

    #[test]
    fn da_and_to_unicode_parsing() {
        let da = parse_da("/Helv 12 Tf 1 0 0 rg");
        assert_eq!(da.font.as_deref(), Some("Helv"));
        assert_eq!(da.size, 12.0);
        assert_eq!(da.color, "1 0 0 rg");
        let cmap = b"2 beginbfchar <0003> <0020> <0024> <0041> endbfchar\n\
            1 beginbfrange <0044> <0046> <0061> endbfrange\n\
            1 beginbfrange <0050> <0051> [<00E9> <65E5>] endbfrange";
        let m = parse_to_unicode(cmap);
        assert_eq!(m.get(&'A'), Some(&0x24));
        assert_eq!(m.get(&'c'), Some(&0x46));
        assert_eq!(m.get(&'日'), Some(&0x51));
    }

    #[test]
    fn oversized_documents_are_refused() {
        let big: Arc<[u8]> = vec![0u8; (MAX_EDIT_BYTES + 1) as usize].into();
        assert!(matches!(
            FormDoc::open(big, None),
            Err(PdfError::TooLarge(_))
        ));
    }
}
