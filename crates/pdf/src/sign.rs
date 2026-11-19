//! Digital signing (design D9): a PAdES baseline B-B signature (`ETSI.CAdES.detached`)
//! written as its own incremental update on top of whatever else the save contains, so it
//! is always the last change and covers the whole file.
//!
//! The update is serialized once with a fixed-size `/Contents` placeholder and a
//! fixed-width `/ByteRange`; the range is patched in place, the digest taken over the file
//! minus the placeholder, and the CMS SignedData hex-encoded into the placeholder.

use std::sync::Arc;
use std::time::SystemTime;

use der::asn1::{Any, ObjectIdentifier, OctetString, SetOfVec};
use der::{Decode, Encode, Sequence};
use lopdf::{
    Dictionary, Document, IncrementalDocument, LoadOptions, Object, ObjectId, Stream, dictionary,
};
use sha2::Digest as _;
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::Attribute;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::serial_number::SerialNumber;

use crate::cert::{SignKey, Signer};
use crate::doc::find;
use crate::lo::{self, fmt_num};
use crate::secret::Secret;
use crate::strings::encode_text;
use crate::visual::{Visual, check_rect};
use crate::{PdfError, guard};

/// Bytes reserved for the CMS signature (hex-encoded in the file: twice this).
pub const CONTENTS_BYTES: usize = 16 * 1024;
/// Longest reason, location or contact text.
pub const MAX_INFO_CHARS: usize = 256;
/// Width of each patched `/ByteRange` number (files up to 10 GB).
const RANGE_WIDTH: usize = 10;

pub(crate) const OID_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
pub(crate) const OID_SIGNED_DATA: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2");
pub(crate) const OID_CONTENT_TYPE: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.3");
pub(crate) const OID_MESSAGE_DIGEST: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.4");
pub(crate) const OID_SIGNING_TIME: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.5");
pub(crate) const OID_SIGNING_CERT_V2: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.2.47");
pub(crate) const OID_SHA256: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
pub(crate) const OID_SHA384: ObjectIdentifier =
    ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const OID_RSA_ENCRYPTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const OID_ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const OID_ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");

/// What a visible digital signature shows.
#[derive(Clone, Debug, PartialEq)]
pub enum Appearance {
    /// Invisible signature (no widget area).
    None,
    /// A visual signature (drawn, typed or image).
    Visual(Arc<Visual>),
    /// "Digitally signed by NAME" and the date, in Helvetica.
    NameAndDate,
}

/// Where and how to sign.
#[derive(Clone, Debug)]
pub struct Request {
    /// An existing empty signature field to sign, by full name.
    pub field: Option<String>,
    /// Page and page-space rectangle for a new visible signature field.
    pub page: usize,
    pub rect: Option<[f32; 4]>,
    pub appearance: Appearance,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub contact: Option<String>,
    pub time: SystemTime,
}

impl Default for Request {
    fn default() -> Self {
        Self {
            field: None,
            page: 0,
            rect: None,
            appearance: Appearance::None,
            reason: None,
            location: None,
            contact: None,
            time: SystemTime::now(),
        }
    }
}

impl Request {
    pub fn validate(&self) -> Result<(), PdfError> {
        for (label, v) in [
            ("reason", &self.reason),
            ("location", &self.location),
            ("contact", &self.contact),
        ] {
            if v.as_ref()
                .is_some_and(|s| s.chars().count() > MAX_INFO_CHARS)
            {
                return Err(PdfError::Refused(format!(
                    "the {label} is longer than {MAX_INFO_CHARS} characters"
                )));
            }
        }
        Ok(())
    }
}

/// Sign `bytes` (a whole PDF) and return the signed file.
pub fn sign(
    bytes: &[u8],
    doc_password: Option<&Secret>,
    signer: &Signer,
    req: &Request,
) -> Result<Vec<u8>, PdfError> {
    req.validate()?;
    if bytes.len() as u64 > crate::MAX_EDIT_BYTES {
        return Err(PdfError::TooLarge(format!(
            "Signing works on PDFs up to {} MiB.",
            crate::MAX_EDIT_BYTES >> 20
        )));
    }
    guard(|| sign_inner(bytes, doc_password, signer, req))
}

fn sign_inner(
    bytes: &[u8],
    doc_password: Option<&Secret>,
    signer: &Signer,
    req: &Request,
) -> Result<Vec<u8>, PdfError> {
    let pw = doc_password.map(Secret::expose).unwrap_or("");
    let prev = Document::load_mem_with_options(bytes, LoadOptions::with_password(pw))
        .or_else(|_| Document::load_mem(bytes))
        .map_err(|_| PdfError::Unsupported("this PDF cannot be opened for signing".into()))?;
    if prev.is_encrypted() || prev.encryption_state.is_some() {
        return Err(PdfError::Unsupported(
            "digital signatures on encrypted PDFs".into(),
        ));
    }
    let pages = lo::page_ids(&prev);
    let mut inc = IncrementalDocument::create_from(bytes.to_vec(), prev);
    let mut new = Document::new_from_prev(inc.get_prev_documents());
    for key in [
        b"XRefStm".as_slice(),
        b"Type",
        b"W",
        b"Index",
        b"Length",
        b"Filter",
        b"DecodeParms",
        b"Encrypt",
    ] {
        new.trailer.remove(key);
    }
    {
        let prev = inc.get_prev_documents();
        let mut b = Build {
            prev,
            new: &mut new,
        };
        b.add_signature(&pages, signer, req)?;
    }
    inc.new_document = new;
    let mut out = Vec::with_capacity(bytes.len() + CONTENTS_BYTES * 2 + 16 * 1024);
    inc.save_to(&mut out)
        .map_err(|e| PdfError::Unsupported(format!("writing the signature failed ({e})")))?;
    finish(out, bytes.len(), signer, req.time)
}

struct Build<'a> {
    prev: &'a Document,
    new: &'a mut Document,
}

impl Build<'_> {
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
            _ => Err(PdfError::Invalid("an object is not a dictionary".into())),
        }
    }

    fn add_signature(
        &mut self,
        pages: &[ObjectId],
        signer: &Signer,
        req: &Request,
    ) -> Result<(), PdfError> {
        let range_placeholder: Vec<Object> = vec![
            Object::Integer(0),
            Object::Integer(9_999_999_999),
            Object::Integer(9_999_999_999),
            Object::Integer(9_999_999_999),
        ];
        let mut sig = dictionary! {
            "Type" => "Sig",
            "Filter" => "Adobe.PPKLite",
            "SubFilter" => "ETSI.CAdES.detached",
            "ByteRange" => range_placeholder,
            "Contents" => crate::write::hex_string(vec![0u8; CONTENTS_BYTES]),
            "M" => Object::string_literal(pdf_date(req.time)),
            "Name" => Object::String(encode_text(&signer.info.name), lopdf::StringFormat::Literal),
        };
        for (key, v) in [
            ("Reason", &req.reason),
            ("Location", &req.location),
            ("ContactInfo", &req.contact),
        ] {
            if let Some(v) = v.as_ref().filter(|s| !s.trim().is_empty()) {
                sig.set(
                    key,
                    Object::String(encode_text(v), lopdf::StringFormat::Literal),
                );
            }
        }
        let sig_id = self.new.add_object(sig);
        let acro = self.acroform()?;
        if let Some(name) = &req.field {
            let (field_id, widget_id, page_ix) = self.find_sig_field(name, pages)?;
            let rect = lo::get_rect(
                self.prev,
                self.prev
                    .get_dictionary(widget_id)
                    .map_err(|_| PdfError::Invalid("a field widget is missing".into()))?,
                b"Rect",
            );
            self.edit(field_id)?.set("V", sig_id);
            if let (Some(r), false) = (rect, req.appearance == Appearance::None) {
                let (w, h) = ((r[2] - r[0]).max(1.0), (r[3] - r[1]).max(1.0));
                let ap = self.appearance(signer, req, w, h);
                self.edit(widget_id)?.set("AP", dictionary! { "N" => ap });
            }
            let _ = page_ix;
        } else {
            let page_id = *pages.get(req.page).ok_or_else(|| {
                PdfError::Refused(format!("page {} does not exist", req.page + 1))
            })?;
            let g = lo::geometry(self.prev, page_id)?;
            let (rect, ap) = match req.rect {
                Some(r) if req.appearance != Appearance::None => {
                    check_rect(r, (g.width as f32, g.height as f32))?;
                    let u = g.rect_to_user(r);
                    let (w, h) = ((u[2] - u[0]).max(1.0), (u[3] - u[1]).max(1.0));
                    (u, Some(self.appearance(signer, req, w, h)))
                }
                _ => ([0.0; 4], None),
            };
            let n = self.signature_count() + 1;
            let mut widget = dictionary! {
                "Type" => "Annot", "Subtype" => "Widget", "FT" => "Sig",
                "T" => Object::string_literal(format!("Signature{n}")),
                "V" => sig_id,
                "F" => 132,
                "P" => page_id,
                "Rect" => rect.iter().map(|v| Object::Real(*v as f32)).collect::<Vec<_>>(),
            };
            if let Some(ap) = ap {
                widget.set("AP", dictionary! { "N" => ap });
            }
            let widget_id = self.new.add_object(widget);
            self.append_annot(page_id, widget_id)?;
            self.append_field(acro, widget_id)?;
        }
        if let Some(acro) = acro {
            self.acro_dict_mut(acro)?.set("SigFlags", 3);
        }
        Ok(())
    }

    fn signature_count(&self) -> usize {
        self.prev
            .objects
            .values()
            .filter(|o| {
                matches!(o, Object::Dictionary(d) if d.get(b"FT").ok().and_then(|n| n.as_name().ok()) == Some(b"Sig".as_slice()))
            })
            .count()
    }

    fn find_sig_field(
        &self,
        name: &str,
        pages: &[ObjectId],
    ) -> Result<(ObjectId, ObjectId, usize), PdfError> {
        // Walk the AcroForm field tree directly (the document is already parsed).
        let catalog = self
            .prev
            .catalog()
            .map_err(|_| PdfError::Invalid("the catalog is missing".into()))?;
        let acro = lo::get_dict(self.prev, catalog, b"AcroForm")
            .ok_or_else(|| PdfError::Refused(format!("there is no field named “{name}”")))?;
        let Some(Object::Array(fields)) = lo::get(self.prev, acro, b"Fields") else {
            return Err(PdfError::Refused(format!(
                "there is no field named “{name}”"
            )));
        };
        let mut stack: Vec<(ObjectId, String, usize)> = fields
            .iter()
            .filter_map(|f| f.as_reference().ok())
            .map(|id| (id, String::new(), 0))
            .collect();
        let mut visited = 0usize;
        while let Some((id, prefix, depth)) = stack.pop() {
            visited += 1;
            if visited > crate::doc::MAX_FIELDS || depth > crate::doc::MAX_TREE_DEPTH {
                break;
            }
            let Ok(d) = self.prev.get_dictionary(id) else {
                continue;
            };
            let full = match lo::get_text(self.prev, d, b"T") {
                Some(t) if prefix.is_empty() => t,
                Some(t) => format!("{prefix}.{t}"),
                None => prefix.clone(),
            };
            if full == name {
                if lo::get_name(self.prev, d, b"FT").as_deref() != Some("Sig") {
                    return Err(PdfError::Refused(format!(
                        "“{name}” is not a signature field"
                    )));
                }
                if d.has(b"V") {
                    return Err(PdfError::Refused(format!("“{name}” is already signed")));
                }
                let widget = match lo::get(self.prev, d, b"Kids") {
                    Some(Object::Array(k)) => {
                        k.first().and_then(|o| o.as_reference().ok()).unwrap_or(id)
                    }
                    _ => id,
                };
                let page = self
                    .prev
                    .get_dictionary(widget)
                    .ok()
                    .and_then(|w| w.get(b"P").ok())
                    .and_then(|p| p.as_reference().ok())
                    .and_then(|p| pages.iter().position(|x| *x == p))
                    .unwrap_or(0);
                return Ok((id, widget, page));
            }
            if let Some(Object::Array(kids)) = lo::get(self.prev, d, b"Kids") {
                for k in kids.iter().filter_map(|o| o.as_reference().ok()) {
                    stack.push((k, full.clone(), depth + 1));
                }
            }
        }
        Err(PdfError::Refused(format!(
            "there is no field named “{name}”"
        )))
    }

    /// `/AcroForm`: an object id, or the catalog id when it is inline (or must be made).
    fn acroform(&mut self) -> Result<Option<AcroLoc>, PdfError> {
        let root = self
            .prev
            .trailer
            .get(b"Root")
            .ok()
            .and_then(|r| r.as_reference().ok())
            .ok_or_else(|| PdfError::Invalid("the catalog is missing".into()))?;
        let catalog = self
            .prev
            .get_dictionary(root)
            .map_err(|_| PdfError::Invalid("the catalog is missing".into()))?;
        Ok(Some(match catalog.get(b"AcroForm") {
            Ok(Object::Reference(id)) => AcroLoc::Object(*id),
            Ok(Object::Dictionary(_)) => AcroLoc::Inline(root),
            _ => {
                let id = self
                    .new
                    .add_object(dictionary! { "Fields" => Vec::<Object>::new() });
                self.edit(root)?.set("AcroForm", id);
                AcroLoc::Object(id)
            }
        }))
    }

    fn acro_dict_mut(&mut self, loc: AcroLoc) -> Result<&mut Dictionary, PdfError> {
        match loc {
            AcroLoc::Object(id) => self.edit(id),
            AcroLoc::Inline(root) => match self.edit(root)?.get_mut(b"AcroForm") {
                Ok(Object::Dictionary(d)) => Ok(d),
                _ => Err(PdfError::Invalid("the form dictionary is damaged".into())),
            },
        }
    }

    fn append_field(&mut self, acro: Option<AcroLoc>, field: ObjectId) -> Result<(), PdfError> {
        let Some(acro) = acro else {
            return Ok(());
        };
        let fields_ref = match self.acro_dict_mut(acro)?.get(b"Fields") {
            Ok(Object::Reference(r)) => Some(*r),
            _ => None,
        };
        if let Some(r) = fields_ref {
            let arr = match self.prev.get_object(r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            let mut arr = match self.new.get_object(r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => arr,
            };
            arr.push(Object::Reference(field));
            self.new.set_object(r, Object::Array(arr));
            return Ok(());
        }
        let d = self.acro_dict_mut(acro)?;
        let mut arr = match d.get(b"Fields") {
            Ok(Object::Array(a)) => a.clone(),
            _ => Vec::new(),
        };
        arr.push(Object::Reference(field));
        d.set("Fields", arr);
        Ok(())
    }

    fn append_annot(&mut self, page: ObjectId, annot: ObjectId) -> Result<(), PdfError> {
        let annots = self
            .prev
            .get_dictionary(page)
            .ok()
            .and_then(|p| p.get(b"Annots").ok())
            .cloned();
        match annots {
            Some(Object::Reference(r)) => {
                let mut arr = match self.prev.get_object(r) {
                    Ok(Object::Array(a)) => a.clone(),
                    _ => Vec::new(),
                };
                arr.push(Object::Reference(annot));
                self.new.set_object(r, Object::Array(arr));
            }
            Some(Object::Array(mut a)) => {
                a.push(Object::Reference(annot));
                self.edit(page)?.set("Annots", a);
            }
            _ => {
                self.edit(page)?
                    .set("Annots", vec![Object::Reference(annot)]);
            }
        }
        Ok(())
    }

    /// A Form XObject for the visible signature.
    fn appearance(&mut self, signer: &Signer, req: &Request, w: f64, h: f64) -> ObjectId {
        let mut resources = Dictionary::new();
        let content = match &req.appearance {
            Appearance::Visual(v) => {
                if let Some(img) = &v.image {
                    let mut d = dictionary! {
                        "Type" => "XObject", "Subtype" => "Image",
                        "Width" => i64::from(img.width), "Height" => i64::from(img.height),
                        "ColorSpace" => img.color_space, "BitsPerComponent" => 8,
                        "Filter" => img.filter,
                    };
                    if let Some(mask) = &img.smask {
                        let m = self.new.add_object(
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
                        d.set("SMask", m);
                    }
                    let im = self
                        .new
                        .add_object(Stream::new(d, img.data.clone()).with_compression(false));
                    resources.set("XObject", dictionary! { "Im0" => im });
                }
                v.fitted(w, h)
            }
            _ => {
                let font = self.new.add_object(dictionary! {
                    "Type" => "Font", "Subtype" => "Type1",
                    "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
                });
                resources.set("Font", dictionary! { "Helv" => font });
                let size = (h / 3.2).clamp(4.0, 12.0);
                let date = human_date(req.time);
                let lines = [
                    format!("Digitally signed by {}", signer.info.name),
                    format!("Date: {date}"),
                ];
                let mut s = format!("BT /Helv {} Tf 0 g\n", fmt_num(size));
                for (i, line) in lines.iter().enumerate() {
                    let bytes: Vec<u8> = line
                        .chars()
                        .filter_map(crate::strings::winansi_byte)
                        .collect();
                    s.push_str(&format!(
                        "1 0 0 1 2 {} Tm {} Tj\n",
                        fmt_num(h - size * (1.2 + 1.3 * i as f64)),
                        lo::literal(&bytes)
                    ));
                }
                s.push_str("ET\n");
                s.into_bytes()
            }
        };
        self.new.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Form",
                "BBox" => vec![0.into(), 0.into(), Object::Real(w as f32), Object::Real(h as f32)],
                "Resources" => resources,
            },
            content,
        ))
    }
}

#[derive(Clone, Copy, Debug)]
enum AcroLoc {
    Object(ObjectId),
    Inline(ObjectId),
}

/// `D:YYYYMMDDHHmmSSZ` (UTC).
pub(crate) fn pdf_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, h, mi, s) = civil(secs);
    format!("D:{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z")
}

pub(crate) fn human_date(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, mo, d, h, mi, _) = civil(secs);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

/// Unix seconds → (year, month, day, hour, minute, second) in UTC.
pub(crate) fn civil(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Howard Hinnant's days-from-civil inverse.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (
        y,
        m,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

/// Patch `/ByteRange`, digest, sign and fill `/Contents` in the serialized file.
fn finish(
    mut out: Vec<u8>,
    prev_len: usize,
    signer: &Signer,
    time: SystemTime,
) -> Result<Vec<u8>, PdfError> {
    let tail = out.get(prev_len..).ok_or(PdfError::Internal)?;
    // The placeholder is the only long run of zeros in a hex string of the update.
    let mut marker = vec![b'<'];
    marker.extend(std::iter::repeat_n(b'0', 64));
    let contents_at = find(tail, &marker).ok_or(PdfError::Internal)?;
    let start = prev_len + contents_at;
    let end = start + 2 + CONTENTS_BYTES * 2;
    if out.get(end - 1) != Some(&b'>') {
        return Err(PdfError::Internal);
    }
    let total = out.len();
    let range = [0usize, start, end, total - end];
    // `/ByteRange [0 9999999999 9999999999 9999999999]`: rewrite inside the brackets.
    let br_rel = find(tail, b"/ByteRange").ok_or(PdfError::Internal)?;
    let br = prev_len + br_rel;
    let open = br + find(out.get(br..).unwrap_or(&[]), b"[").ok_or(PdfError::Internal)?;
    let close = open + find(out.get(open..).unwrap_or(&[]), b"]").ok_or(PdfError::Internal)?;
    let width = close - open - 1;
    let mut text = format!(
        "{} {:<w$} {:<w$} {:<w$}",
        range[0],
        range[1],
        range[2],
        range[3],
        w = RANGE_WIDTH
    );
    if text.len() > width {
        return Err(PdfError::TooLarge("the file is too large to sign".into()));
    }
    while text.len() < width {
        text.push(' ');
    }
    out.splice(open + 1..close, text.into_bytes());
    // Digest over everything except the placeholder (including its angle brackets? No:
    // the brackets are inside the excluded range [start, end)).
    let (digest_oid, digest) = match &signer.key {
        SignKey::P384(_) => {
            let mut h = sha2::Sha384::new();
            h.update(&out[..start]);
            h.update(&out[end..]);
            (OID_SHA384, h.finalize().to_vec())
        }
        _ => {
            let mut h = sha2::Sha256::new();
            h.update(&out[..start]);
            h.update(&out[end..]);
            (OID_SHA256, h.finalize().to_vec())
        }
    };
    let cms = signed_data(signer, digest_oid, &digest, time)?;
    if cms.len() > CONTENTS_BYTES {
        return Err(PdfError::TooLarge(
            "the signature is larger than the space reserved for it (certificate chain too long)"
                .into(),
        ));
    }
    let hex: Vec<u8> = cms
        .iter()
        .flat_map(|b| format!("{b:02X}").into_bytes())
        .collect();
    out[start + 1..start + 1 + hex.len()].copy_from_slice(&hex);
    Ok(out)
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct IssuerSerial {
    pub issuer: Vec<GeneralName>,
    pub serial_number: SerialNumber,
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct EssCertIdV2 {
    // hashAlgorithm is DEFAULT sha256 and therefore omitted.
    pub cert_hash: OctetString,
    pub issuer_serial: IssuerSerial,
}

#[derive(Clone, Debug, Eq, PartialEq, Sequence)]
pub(crate) struct SigningCertificateV2 {
    pub certs: Vec<EssCertIdV2>,
}

fn attr(oid: ObjectIdentifier, value: Any) -> Result<Attribute, PdfError> {
    let mut values = SetOfVec::new();
    values.insert(value).map_err(|_| PdfError::Internal)?;
    Ok(Attribute { oid, values })
}

fn der_err<E>(_: E) -> PdfError {
    PdfError::Internal
}

/// The CAdES detached SignedData.
fn signed_data(
    signer: &Signer,
    digest_oid: ObjectIdentifier,
    digest: &[u8],
    time: SystemTime,
) -> Result<Vec<u8>, PdfError> {
    let cert: &Certificate = &signer.cert;
    let cert_der = cert.to_der().map_err(der_err)?;
    let ess = SigningCertificateV2 {
        certs: vec![EssCertIdV2 {
            cert_hash: OctetString::new(sha2::Sha256::digest(&cert_der).to_vec())
                .map_err(der_err)?,
            issuer_serial: IssuerSerial {
                issuer: vec![GeneralName::DirectoryName(
                    cert.tbs_certificate().issuer().clone(),
                )],
                serial_number: cert.tbs_certificate().serial_number().clone(),
            },
        }],
    };
    let signing_time = x509_cert::time::Time::try_from(time).map_err(der_err)?;
    let mut attrs = SetOfVec::new();
    for a in [
        attr(
            OID_CONTENT_TYPE,
            Any::encode_from(&OID_DATA).map_err(der_err)?,
        )?,
        attr(
            OID_SIGNING_TIME,
            Any::encode_from(&signing_time).map_err(der_err)?,
        )?,
        attr(
            OID_MESSAGE_DIGEST,
            Any::encode_from(&OctetString::new(digest.to_vec()).map_err(der_err)?)
                .map_err(der_err)?,
        )?,
        attr(
            OID_SIGNING_CERT_V2,
            Any::encode_from(&ess).map_err(der_err)?,
        )?,
    ] {
        attrs.insert(a).map_err(der_err)?;
    }
    let to_sign = attrs.to_der().map_err(der_err)?;
    let (sig_alg, signature) = sign_bytes(&signer.key, &to_sign)?;
    let digest_alg = AlgorithmIdentifierOwned {
        oid: digest_oid,
        parameters: None,
    };
    let signer_info = cms::signed_data::SignerInfo {
        version: cms::content_info::CmsVersion::V1,
        sid: cms::signed_data::SignerIdentifier::IssuerAndSerialNumber(
            cms::cert::IssuerAndSerialNumber {
                issuer: cert.tbs_certificate().issuer().clone(),
                serial_number: cert.tbs_certificate().serial_number().clone(),
            },
        ),
        digest_alg: digest_alg.clone(),
        signed_attrs: Some(attrs),
        signature_algorithm: sig_alg,
        signature: OctetString::new(signature).map_err(der_err)?,
        unsigned_attrs: None,
    };
    let mut certs = SetOfVec::new();
    certs
        .insert(cms::cert::CertificateChoices::Certificate(cert.clone()))
        .map_err(der_err)?;
    for c in &signer.chain {
        // Duplicates are skipped by the set.
        let _ = certs.insert(cms::cert::CertificateChoices::Certificate(c.clone()));
    }
    let mut digest_algs = SetOfVec::new();
    digest_algs.insert(digest_alg).map_err(der_err)?;
    let mut infos = SetOfVec::new();
    infos.insert(signer_info).map_err(der_err)?;
    let sd = cms::signed_data::SignedData {
        version: cms::content_info::CmsVersion::V1,
        digest_algorithms: digest_algs,
        encap_content_info: cms::signed_data::EncapsulatedContentInfo {
            econtent_type: OID_DATA,
            econtent: None,
        },
        certificates: Some(cms::signed_data::CertificateSet(certs)),
        crls: None,
        signer_infos: cms::signed_data::SignerInfos(infos),
    };
    let ci = cms::content_info::ContentInfo {
        content_type: OID_SIGNED_DATA,
        content: Any::encode_from(&sd).map_err(der_err)?,
    };
    ci.to_der().map_err(der_err)
}

fn sign_bytes(key: &SignKey, msg: &[u8]) -> Result<(AlgorithmIdentifierOwned, Vec<u8>), PdfError> {
    use signature::{RandomizedSigner, SignatureEncoding, Signer as _};
    match key {
        SignKey::Rsa(k) => {
            let sk = rsa::pkcs1v15::SigningKey::<sha2::Sha256>::new((**k).clone());
            let mut rng = getrandom::SysRng;
            let sig = sk
                .try_sign_with_rng(&mut rng, msg)
                .map_err(|_| PdfError::Certificate("signing failed".into()))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: OID_RSA_ENCRYPTION,
                    parameters: Some(Any::null()),
                },
                sig.to_vec(),
            ))
        }
        SignKey::P256(k) => {
            let sig: p256::ecdsa::Signature = k
                .try_sign(msg)
                .map_err(|_| PdfError::Certificate("signing failed".into()))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: OID_ECDSA_SHA256,
                    parameters: None,
                },
                sig.to_der().to_vec(),
            ))
        }
        SignKey::P384(k) => {
            let sig: p384::ecdsa::Signature = k
                .try_sign(msg)
                .map_err(|_| PdfError::Certificate("signing failed".into()))?;
            Ok((
                AlgorithmIdentifierOwned {
                    oid: OID_ECDSA_SHA384,
                    parameters: None,
                },
                sig.to_der().to_vec(),
            ))
        }
    }
}

/// Decode a DER ContentInfo holding SignedData (shared with verification).
pub(crate) fn parse_signed_data(der: &[u8]) -> Option<cms::signed_data::SignedData> {
    let ci = cms::content_info::ContentInfo::from_der(der).ok()?;
    if ci.content_type != OID_SIGNED_DATA {
        return None;
    }
    ci.content.decode_as::<cms::signed_data::SignedData>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cert::load_pkcs12;
    use crate::fixtures::{self, certs};
    use crate::verify::{SigStatus, verify_all};

    fn signer(key: certs::KeySpec) -> Signer {
        let p12 = certs::p12(&certs::P12Spec {
            key,
            ..Default::default()
        });
        load_pkcs12(&p12, &Secret::new("test-pass"), SystemTime::now()).expect("signer")
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(civil(1_763_553_600), (2025, 11, 19, 12, 0, 0));
        assert_eq!(civil(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn signs_and_verifies_with_each_key_type() {
        for key in [
            certs::KeySpec::P256,
            certs::KeySpec::P384,
            certs::KeySpec::Rsa2048,
        ] {
            let original = fixtures::text_pages(2);
            let s = signer(key);
            let req = Request {
                rect: Some([50.0, 700.0, 300.0, 780.0]),
                appearance: Appearance::NameAndDate,
                reason: Some("Approved".into()),
                location: Some("Amsterdam".into()),
                ..Default::default()
            };
            let out = sign(&original, None, &s, &req).expect("sign");
            assert!(out.starts_with(&original));
            let doc = crate::Document::open(out.clone().into(), None).expect("reopen");
            assert_eq!(doc.info().forms.signed, 1);
            let reports = verify_all(&doc);
            assert_eq!(reports.len(), 1, "{key:?}");
            let r = &reports[0];
            assert_eq!(r.status, SigStatus::Valid, "{key:?}: {r:?}");
            assert_eq!(r.reason.as_deref(), Some("Approved"));
            assert_eq!(r.location.as_deref(), Some("Amsterdam"));
            assert_eq!(
                r.signer.as_ref().map(|c| c.name.as_str()),
                Some("Test Signer")
            );
            assert!(r.weak.is_empty(), "{key:?}: {:?}", r.weak);
            // The visible appearance is drawn.
            let t = crate::text::page_text(&doc, 0).expect("text");
            assert!(
                t.text.contains("Digitally signed by Test Signer"),
                "{:?}",
                t.text
            );
        }
    }

    #[test]
    fn signs_an_existing_field_after_filling() {
        let mut form = crate::forms::FormDoc::open(fixtures::form().into(), None).expect("form");
        form.set("name", crate::forms::FieldInput::Text("Jane".into()))
            .expect("fill");
        let filled = form.build(&[]).expect("build");
        let s = signer(certs::KeySpec::P256);
        let req = Request {
            field: Some("sig".into()),
            appearance: Appearance::NameAndDate,
            ..Default::default()
        };
        let out = sign(&filled, None, &s, &req).expect("sign");
        let doc = crate::Document::open(out.clone().into(), None).expect("reopen");
        let r = verify_all(&doc);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].status, SigStatus::Valid, "{:?}", r[0]);
        assert_eq!(r[0].field, "sig");
        // Signing the same field again is refused.
        assert!(sign(&out, None, &s, &req).is_err());
        let again = crate::forms::FormDoc::open(out.into(), None).expect("lopdf");
        assert_eq!(
            again.field("name").map(|f| &f.value),
            Some(&crate::forms::FieldValue::Text("Jane".into()))
        );
    }

    #[test]
    fn second_signature_keeps_the_first_valid() {
        let s = signer(certs::KeySpec::P256);
        let once = sign(&fixtures::text_pages(1), None, &s, &Request::default()).expect("one");
        let twice = sign(&once, None, &s, &Request::default()).expect("two");
        let doc = crate::Document::open(twice.into(), None).expect("reopen");
        let mut r = verify_all(&doc);
        r.sort_by(|a, b| a.field.cmp(&b.field));
        assert_eq!(r.len(), 2);
        assert!(
            matches!(&r[0].status, SigStatus::ChangedAfter(c) if c.iter().any(|k| k.contains("signature"))),
            "{:?}",
            r[0]
        );
        assert_eq!(r[1].status, SigStatus::Valid);
    }

    #[test]
    fn refusals() {
        let s = signer(certs::KeySpec::P256);
        let long = Request {
            reason: Some("x".repeat(MAX_INFO_CHARS + 1)),
            ..Default::default()
        };
        assert!(sign(&fixtures::text_pages(1), None, &s, &long).is_err());
        let outside = Request {
            rect: Some([500.0, 10.0, 900.0, 50.0]),
            appearance: Appearance::NameAndDate,
            ..Default::default()
        };
        assert!(sign(&fixtures::text_pages(1), None, &s, &outside).is_err());
        let enc = fixtures::encrypted("pw");
        let err =
            sign(&enc, Some(&Secret::new("pw")), &s, &Request::default()).expect_err("encrypted");
        assert!(err.to_string().contains("encrypted"));
        let missing = Request {
            field: Some("nope".into()),
            ..Default::default()
        };
        assert!(sign(&fixtures::form(), None, &s, &missing).is_err());
    }
}
