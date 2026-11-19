//! Opening a document and describing it: pages, labels, outline, links, metadata,
//! encryption and a summary of its form and signature fields.
//!
//! Coordinates handed out are in *page space*: points, origin at the top-left of the
//! page as displayed (crop box, `/Rotate` applied), y growing downwards.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::object::{Array, Dict, MaybeRef, Name, Object, ObjectIdentifier};
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{DecryptionError, LoadPdfError, Pdf};
use hayro::kurbo::{Affine, Point};

use crate::secret::Secret;
use crate::strings::{decode_text, letters, roman};
use crate::{MAX_FILE_BYTES, MAX_PAGE_PT, MAX_PAGES, PdfError, guard};

/// Outline entries kept.
pub const MAX_OUTLINE: usize = 10_000;
/// Outline nesting kept.
pub const MAX_OUTLINE_DEPTH: u8 = 16;
/// Links read per page.
pub const MAX_LINKS: usize = 2_000;
/// Fields visited when summarizing a form.
pub const MAX_FIELDS: usize = 10_000;
/// Nesting of the field tree, name trees and number trees.
pub const MAX_TREE_DEPTH: usize = 32;

/// Why a document did not open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The document needs a user password (none or a wrong one was given).
    NeedsPassword { wrong: bool },
    /// Larger than [`MAX_FILE_BYTES`].
    TooLarge,
    /// Not readable as a PDF (reason).
    Invalid(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsPassword { wrong: false } => f.write_str("this PDF is password-protected"),
            Self::NeedsPassword { wrong: true } => f.write_str("incorrect password"),
            Self::TooLarge => write!(
                f,
                "the PDF is larger than {} MiB and is not previewed",
                MAX_FILE_BYTES >> 20
            ),
            Self::Invalid(why) => write!(f, "the PDF could not be read: {why}"),
        }
    }
}

/// A page as displayed.
#[derive(Clone, Debug, PartialEq)]
pub struct PageInfo {
    /// Width and height in points after cropping and rotation.
    pub width: f32,
    pub height: f32,
    /// The page label (`/PageLabels`), when the document defines one.
    pub label: Option<String>,
}

/// Where a link or outline entry goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dest {
    /// Page index (0-based).
    pub page: usize,
    /// Distance from the top of the page in page-space points, when the destination
    /// names one.
    pub top: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineItem {
    pub title: String,
    pub depth: u8,
    pub dest: Option<Dest>,
    /// Whether the next item is a child of this one.
    pub has_children: bool,
}

/// What clicking a link does.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    Dest(Dest),
    /// A web address; opened only through the app's explicit link path.
    Uri(String),
    /// An action that is never performed (Launch, GoToR, JavaScript, SubmitForm, …).
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// `[x0, y0, x1, y1]` in page space.
    pub rect: [f32; 4],
    pub target: LinkTarget,
}

/// The document's permission flags (ISO 32000-1 Table 22), when it is encrypted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub raw: i64,
}

impl Permissions {
    fn bit(self, n: u32) -> bool {
        self.raw & (1 << (n - 1)) != 0
    }
    pub fn print(self) -> bool {
        self.bit(3)
    }
    pub fn modify(self) -> bool {
        self.bit(4)
    }
    pub fn copy(self) -> bool {
        self.bit(5)
    }
    pub fn annotate(self) -> bool {
        self.bit(6)
    }
    /// Filling form fields (bit 9, or bit 6 which implies it).
    pub fn fill_forms(self) -> bool {
        self.bit(9) || self.bit(6)
    }

    /// Plain-language list of what is restricted (empty when nothing is).
    pub fn restrictions(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.print() {
            out.push("printing");
        }
        if !self.modify() {
            out.push("changing the document");
        }
        if !self.copy() {
            out.push("copying text");
        }
        if !self.annotate() {
            out.push("comments");
        }
        if !self.fill_forms() {
            out.push("filling forms");
        }
        out
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum XfaKind {
    #[default]
    None,
    /// XFA plus AcroForm fields: filled through AcroForm, XFA dropped on save.
    Hybrid,
    /// XFA only (dynamic form): not fillable here.
    Only,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormSummary {
    /// Terminal fields other than signatures.
    pub fields: usize,
    /// Signature fields (signed or empty).
    pub signature_fields: usize,
    /// Signature fields that carry a signature value.
    pub signed: usize,
    pub xfa: XfaKind,
    /// Some field has JavaScript actions (never run).
    pub scripts: bool,
    /// A certification signature forbids changes (DocMDP P=1).
    pub certified_no_changes: bool,
}

/// Everything the viewer needs to know without rendering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DocInfo {
    pub version: String,
    pub pages: Vec<PageInfo>,
    /// The document has more pages than [`MAX_PAGES`]; later ones are not shown.
    pub pages_truncated: bool,
    pub outline: Vec<OutlineItem>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub encrypted: bool,
    /// Opened with a user password the user typed.
    pub password_used: bool,
    pub permissions: Option<Permissions>,
    pub forms: FormSummary,
}

impl DocInfo {
    /// Index of the page whose label (or 1-based number) is `text`.
    pub fn find_page(&self, text: &str) -> Option<usize> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if let Some(ix) = self
            .pages
            .iter()
            .position(|p| p.label.as_deref() == Some(text))
        {
            return Some(ix);
        }
        let n: usize = text.parse().ok()?;
        (1..=self.pages.len()).contains(&n).then(|| n - 1)
    }

    /// Rows for the information section.
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut rows = vec![
            ("PDF version".to_string(), self.version.clone()),
            ("Pages".to_string(), self.pages.len().to_string()),
        ];
        for (label, value) in [
            ("Title", &self.title),
            ("Author", &self.author),
            ("Subject", &self.subject),
            ("Creator", &self.creator),
            ("Producer", &self.producer),
        ] {
            if let Some(v) = value.as_ref().filter(|v| !v.trim().is_empty()) {
                rows.push((label.to_string(), v.clone()));
            }
        }
        let encryption = match (self.encrypted, self.password_used) {
            (false, _) => "None".to_string(),
            (true, true) => "Password-protected".to_string(),
            (true, false) => "Encrypted (opens without a password)".to_string(),
        };
        rows.push(("Encryption".into(), encryption));
        if let Some(p) = self.permissions {
            let r = p.restrictions();
            if !r.is_empty() {
                rows.push(("Restricted".into(), r.join(", ")));
            }
        }
        if self.forms.fields > 0 {
            rows.push(("Form fields".into(), self.forms.fields.to_string()));
        }
        if self.forms.signature_fields > 0 {
            rows.push((
                "Signatures".into(),
                format!(
                    "{} signed, {} empty",
                    self.forms.signed,
                    self.forms.signature_fields - self.forms.signed
                ),
            ));
        }
        rows
    }
}

/// An open document. Cheap to share (`Arc`), safe to use from several threads.
pub struct Document {
    bytes: Arc<[u8]>,
    pdf: Pdf,
    info: DocInfo,
    /// Page object → index, for destinations.
    page_ids: HashMap<ObjectIdentifier, usize>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("bytes", &self.bytes.len())
            .field("pages", &self.info.pages.len())
            .finish_non_exhaustive()
    }
}

impl Document {
    /// Open `bytes`, with a user password when the document needs one.
    pub fn open(bytes: Arc<[u8]>, password: Option<&Secret>) -> Result<Self, OpenError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(OpenError::TooLarge);
        }
        let result = guard(|| Ok(Self::open_inner(bytes, password)));
        match result {
            Ok(r) => r,
            Err(_) => Err(OpenError::Invalid("the file structure is damaged".into())),
        }
    }

    fn open_inner(bytes: Arc<[u8]>, password: Option<&Secret>) -> Result<Self, OpenError> {
        let pw = password.map(Secret::expose).unwrap_or("");
        let data: Arc<Vec<u8>> = Arc::new(bytes.to_vec());
        let pdf = match Pdf::new_with_password(data, pw) {
            Ok(p) => p,
            Err(LoadPdfError::Decryption(DecryptionError::PasswordProtected)) => {
                return Err(OpenError::NeedsPassword {
                    wrong: password.is_some_and(|p| !p.is_empty()),
                });
            }
            Err(LoadPdfError::Decryption(DecryptionError::UnsupportedAlgorithm)) => {
                return Err(OpenError::Invalid(
                    "it uses an unsupported encryption method".into(),
                ));
            }
            Err(LoadPdfError::Decryption(_)) => {
                return Err(OpenError::Invalid("its encryption is damaged".into()));
            }
            Err(LoadPdfError::Invalid) => {
                return Err(OpenError::Invalid("the file structure is damaged".into()));
            }
        };
        let mut page_ids = HashMap::new();
        let all = pdf.pages();
        let mut pages = Vec::with_capacity(all.len().min(MAX_PAGES));
        for (ix, page) in all.iter().take(MAX_PAGES).enumerate() {
            let (w, h) = page.render_dimensions();
            pages.push(PageInfo {
                width: w.clamp(1.0, MAX_PAGE_PT),
                height: h.clamp(1.0, MAX_PAGE_PT),
                label: None,
            });
            if let Some(id) = page.raw().obj_id() {
                page_ids.insert(id, ix);
            }
        }
        if pages.is_empty() {
            return Err(OpenError::Invalid("it has no pages".into()));
        }
        let meta = pdf.metadata();
        let text = |b: &Option<Box<[u8]>>| b.as_deref().map(decode_text);
        let (encrypted, permissions) = encryption_of(&pdf, &bytes);
        let mut info = DocInfo {
            version: version_text(&pdf),
            pages_truncated: all.len() > MAX_PAGES,
            pages,
            outline: Vec::new(),
            title: text(&meta.title),
            author: text(&meta.author),
            subject: text(&meta.subject),
            creator: text(&meta.creator),
            producer: text(&meta.producer),
            encrypted,
            password_used: password.is_some_and(|p| !p.is_empty()),
            permissions,
            forms: FormSummary::default(),
        };
        let mut doc = Self {
            bytes,
            pdf,
            info: DocInfo::default(),
            page_ids,
        };
        if let Some(catalog) = doc.catalog() {
            apply_labels(&catalog, &mut info.pages);
            info.outline = doc.read_outline(&catalog);
            info.forms = summarize_forms(&catalog);
        }
        doc.info = info;
        Ok(doc)
    }

    pub fn info(&self) -> &DocInfo {
        &self.info
    }

    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    pub fn page_count(&self) -> usize {
        self.info.pages.len()
    }

    pub(crate) fn pdf(&self) -> &Pdf {
        &self.pdf
    }

    pub(crate) fn page(&self, index: usize) -> Result<&Page<'_>, PdfError> {
        if index >= self.info.pages.len() {
            return Err(PdfError::Refused(format!(
                "page {} does not exist",
                index + 1
            )));
        }
        self.pdf
            .pages()
            .get(index)
            .ok_or_else(|| PdfError::Refused(format!("page {} does not exist", index + 1)))
    }

    fn catalog(&self) -> Option<Dict<'_>> {
        let xref = self.pdf.xref();
        xref.get::<Dict<'_>>(xref.root_id())
    }

    /// PDF user space → page space for page `index`.
    pub(crate) fn user_to_page(&self, index: usize) -> Option<Affine> {
        let page = self.pdf.pages().get(index)?;
        Some(page.initial_transform(true).to_kurbo())
    }

    /// Links on page `index`, in page space.
    pub fn links(&self, index: usize) -> Result<Vec<Link>, PdfError> {
        guard(|| {
            let page = self.page(index)?;
            let Some(to_page) = self.user_to_page(index) else {
                return Ok(Vec::new());
            };
            let Some(annots) = page.raw().get::<Array<'_>>(b"Annots") else {
                return Ok(Vec::new());
            };
            let mut out = Vec::new();
            for annot in annots.iter::<Dict<'_>>().take(MAX_LINKS * 4) {
                if out.len() >= MAX_LINKS {
                    break;
                }
                if annot.get::<Name<'_>>(b"Subtype").as_deref() != Some(b"Link".as_slice()) {
                    continue;
                }
                let Some(rect) = annot.get::<[f32; 4]>(b"Rect") else {
                    continue;
                };
                let target = if let Some(action) = annot.get::<Dict<'_>>(b"A") {
                    self.action_target(&action)
                } else if let Some(dest) = annot.get::<Object<'_>>(b"Dest") {
                    match self.resolve_dest(&dest, 0) {
                        Some(d) => LinkTarget::Dest(d),
                        None => continue,
                    }
                } else {
                    continue;
                };
                out.push(Link {
                    rect: map_rect(to_page, rect),
                    target,
                });
            }
            Ok(out)
        })
    }

    fn action_target(&self, action: &Dict<'_>) -> LinkTarget {
        let kind = action
            .get::<Name<'_>>(b"S")
            .map(|n| n.as_str().to_string())
            .unwrap_or_default();
        match kind.as_str() {
            "GoTo" => match action
                .get::<Object<'_>>(b"D")
                .and_then(|d| self.resolve_dest(&d, 0))
            {
                Some(d) => LinkTarget::Dest(d),
                None => LinkTarget::Unsupported("a destination that does not exist".into()),
            },
            "URI" => match action.get::<hayro::hayro_syntax::object::String<'_>>(b"URI") {
                Some(uri) => {
                    let text = String::from_utf8_lossy(uri.as_bytes())
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(2048)
                        .collect::<String>();
                    LinkTarget::Uri(text)
                }
                None => LinkTarget::Unsupported("an empty web link".into()),
            },
            "Named" => {
                let name = action
                    .get::<Name<'_>>(b"N")
                    .map(|n| n.as_str().to_string())
                    .unwrap_or_default();
                let last = self.info.pages.len().saturating_sub(1);
                let page = match name.as_str() {
                    "FirstPage" => Some(0),
                    "LastPage" => Some(last),
                    _ => None,
                };
                match page {
                    Some(page) => LinkTarget::Dest(Dest { page, top: None }),
                    None => LinkTarget::Unsupported(format!("the action “{name}”")),
                }
            }
            "Launch" => LinkTarget::Unsupported("starting a program".into()),
            "GoToR" | "GoToE" => LinkTarget::Unsupported("opening another file".into()),
            "JavaScript" => LinkTarget::Unsupported("running a script".into()),
            "SubmitForm" => LinkTarget::Unsupported("sending form data".into()),
            "ImportData" => LinkTarget::Unsupported("importing data".into()),
            "" => LinkTarget::Unsupported("an unknown action".into()),
            other => LinkTarget::Unsupported(format!("the action “{other}”")),
        }
    }

    /// A destination: an explicit array, a name or a string (named destination).
    fn resolve_dest(&self, dest: &Object<'_>, depth: usize) -> Option<Dest> {
        if depth > 4 {
            return None;
        }
        match dest {
            Object::Array(arr) => self.explicit_dest(arr),
            Object::Name(n) => self.named_dest(n.as_ref(), depth),
            Object::String(s) => self.named_dest(s.as_bytes(), depth),
            Object::Dict(d) => d
                .get::<Object<'_>>(b"D")
                .and_then(|d| self.resolve_dest(&d, depth + 1)),
            _ => None,
        }
    }

    fn explicit_dest(&self, arr: &Array<'_>) -> Option<Dest> {
        let mut items = arr.raw_iter();
        let page = match items.next()? {
            MaybeRef::Ref(r) => *self.page_ids.get(&ObjectIdentifier::from(r))?,
            MaybeRef::NotRef(Object::Number(n)) => {
                let n = n.as_i64();
                usize::try_from(n)
                    .ok()
                    .filter(|n| *n < self.info.pages.len())?
            }
            MaybeRef::NotRef(_) => return None,
        };
        let kind = match items.next() {
            Some(MaybeRef::NotRef(Object::Name(n))) => n.as_str().to_string(),
            _ => String::new(),
        };
        let nums: Vec<Option<f32>> = items
            .take(4)
            .map(|o| match o {
                MaybeRef::NotRef(Object::Number(n)) => Some(n.as_f32()),
                _ => None,
            })
            .collect();
        let at = |i: usize| nums.get(i).copied().flatten();
        let user_point = match kind.as_str() {
            "XYZ" => at(1).map(|top| (at(0).unwrap_or(0.0), top)),
            "FitH" | "FitBH" => at(0).map(|top| (0.0, top)),
            "FitR" => at(3).map(|top| (at(0).unwrap_or(0.0), top)),
            _ => None,
        };
        let top = user_point.and_then(|(x, y)| {
            let to_page = self.user_to_page(page)?;
            let p = to_page * Point::new(x as f64, y as f64);
            let height = self.info.pages.get(page)?.height;
            Some((p.y as f32).clamp(0.0, height))
        });
        Some(Dest { page, top })
    }

    fn named_dest(&self, name: &[u8], depth: usize) -> Option<Dest> {
        let catalog = self.catalog()?;
        // PDF 1.1: /Dests dictionary in the catalog.
        if let Some(dests) = catalog.get::<Dict<'_>>(b"Dests")
            && let Some(obj) = dests.get::<Object<'_>>(name)
        {
            return self.resolve_dest(&obj, depth + 1);
        }
        let tree = catalog
            .get::<Dict<'_>>(b"Names")
            .and_then(|n| n.get::<Dict<'_>>(b"Dests"))?;
        let found = name_tree_lookup(&tree, name, 0, &mut 0)?;
        self.resolve_dest(&found, depth + 1)
    }

    fn read_outline(&self, catalog: &Dict<'_>) -> Vec<OutlineItem> {
        let mut out = Vec::new();
        let Some(root) = catalog.get::<Dict<'_>>(b"Outlines") else {
            return out;
        };
        let mut seen = HashSet::new();
        if let Some(first) = root.get::<Dict<'_>>(b"First") {
            self.outline_level(first, 0, &mut out, &mut seen);
        }
        out
    }

    fn outline_level(
        &self,
        first: Dict<'_>,
        depth: u8,
        out: &mut Vec<OutlineItem>,
        seen: &mut HashSet<ObjectIdentifier>,
    ) {
        let mut item = Some(first);
        while let Some(node) = item.take() {
            if out.len() >= MAX_OUTLINE {
                return;
            }
            if let Some(id) = node.obj_id()
                && !seen.insert(id)
            {
                return;
            }
            let title = node
                .get::<hayro::hayro_syntax::object::String<'_>>(b"Title")
                .map(|s| decode_text(s.as_bytes()))
                .unwrap_or_default();
            let title: String = title.chars().take(512).collect();
            let dest = if let Some(d) = node.get::<Object<'_>>(b"Dest") {
                self.resolve_dest(&d, 0)
            } else {
                node.get::<Dict<'_>>(b"A")
                    .and_then(|a| match self.action_target(&a) {
                        LinkTarget::Dest(d) => Some(d),
                        _ => None,
                    })
            };
            let child = node.get::<Dict<'_>>(b"First");
            out.push(OutlineItem {
                title,
                depth,
                dest,
                has_children: false,
            });
            if let Some(child) = child
                && depth + 1 < MAX_OUTLINE_DEPTH
            {
                let at = out.len();
                self.outline_level(child, depth + 1, out, seen);
                if out.len() > at
                    && let Some(me) = out.get_mut(at - 1)
                {
                    me.has_children = true;
                }
            }
            item = node.get::<Dict<'_>>(b"Next");
        }
    }
}

fn map_rect(to_page: Affine, r: [f32; 4]) -> [f32; 4] {
    let a = to_page * Point::new(r[0] as f64, r[1] as f64);
    let b = to_page * Point::new(r[2] as f64, r[3] as f64);
    [
        a.x.min(b.x) as f32,
        a.y.min(b.y) as f32,
        a.x.max(b.x) as f32,
        a.y.max(b.y) as f32,
    ]
}

fn version_text(pdf: &Pdf) -> String {
    let v = format!("{:?}", pdf.version());
    // `Pdf17` → `1.7`
    let digits: String = v.chars().filter(char::is_ascii_digit).collect();
    match digits.as_bytes() {
        [major, minor, ..] => format!("{}.{}", *major as char, *minor as char),
        _ => v,
    }
}

/// Whether the file is encrypted and its permission flags. hayro does not expose the
/// trailer, so the `/Encrypt` reference is found in the trailer area.
fn encryption_of(pdf: &Pdf, bytes: &[u8]) -> (bool, Option<Permissions>) {
    let tail_start = bytes.len().saturating_sub(64 * 1024);
    let regions = [
        bytes.get(tail_start..).unwrap_or(&[]),
        bytes.get(..bytes.len().min(8 * 1024)).unwrap_or(&[]),
    ];
    for region in regions {
        let mut at = 0;
        while let Some(pos) = find(region.get(at..).unwrap_or(&[]), b"/Encrypt") {
            let start = at + pos + b"/Encrypt".len();
            at = start;
            let rest = region.get(start..).unwrap_or(&[]);
            let rest_text =
                String::from_utf8_lossy(rest.get(..rest.len().min(64)).unwrap_or(&[])).to_string();
            let mut parts = rest_text.split_whitespace();
            let num = parts.next().and_then(|n| n.parse::<u32>().ok());
            let generation = parts.next().and_then(|g| g.parse::<i32>().ok());
            let is_ref = parts.next().is_some_and(|r| r.starts_with('R'));
            if let (Some(n), Some(g), true) = (num, generation, is_ref) {
                let p = pdf
                    .xref()
                    .get::<Dict<'_>>(ObjectIdentifier::new(n, g))
                    .and_then(|d| d.get::<i64>(b"P"))
                    .map(|raw| Permissions { raw });
                return (true, p);
            }
            if rest_text.trim_start().starts_with("<<") {
                let p = rest_text
                    .split("/P")
                    .nth(1)
                    .and_then(|s| s.split_whitespace().next())
                    .and_then(|s| s.trim_end_matches(['>', '/']).parse::<i64>().ok())
                    .map(|raw| Permissions { raw });
                return (true, p);
            }
        }
    }
    (false, None)
}

/// First position of `needle` in `hay`.
pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
pub(crate) fn rfind(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).rposition(|w| w == needle)
}

/// Look `key` up in a name tree.
fn name_tree_lookup<'a>(
    node: &Dict<'a>,
    key: &[u8],
    depth: usize,
    visited: &mut usize,
) -> Option<Object<'a>> {
    *visited += 1;
    if depth > MAX_TREE_DEPTH || *visited > 10_000 {
        return None;
    }
    if let Some(names) = node.get::<Array<'a>>(b"Names") {
        let mut it = names.flex_iter();
        for _ in 0..100_000 {
            let Some(k) = it.next::<hayro::hayro_syntax::object::String<'a>>() else {
                break;
            };
            let Some(v) = it.next::<Object<'a>>() else {
                break;
            };
            if k.as_bytes() == key {
                return Some(v);
            }
        }
    }
    if let Some(kids) = node.get::<Array<'a>>(b"Kids") {
        for kid in kids.iter::<Dict<'a>>() {
            if let Some(limits) = kid.get::<Array<'a>>(b"Limits") {
                let mut l = limits.iter::<hayro::hayro_syntax::object::String<'a>>();
                if let (Some(lo), Some(hi)) = (l.next(), l.next())
                    && (key < lo.as_bytes() || key > hi.as_bytes())
                {
                    continue;
                }
            }
            if let Some(found) = name_tree_lookup(&kid, key, depth + 1, visited) {
                return Some(found);
            }
        }
    }
    None
}

/// `/PageLabels` number tree → labels.
fn apply_labels(catalog: &Dict<'_>, pages: &mut [PageInfo]) {
    let Some(tree) = catalog.get::<Dict<'_>>(b"PageLabels") else {
        return;
    };
    let mut ranges: Vec<(usize, Dict<'_>)> = Vec::new();
    collect_numbers(&tree, 0, &mut ranges);
    ranges.sort_by_key(|(start, _)| *start);
    for (i, (start, spec)) in ranges.iter().enumerate() {
        let end = ranges.get(i + 1).map_or(pages.len(), |(s, _)| *s);
        let style = spec.get::<Name<'_>>(b"S").map(|n| n.as_str().to_string());
        let prefix = spec
            .get::<hayro::hayro_syntax::object::String<'_>>(b"P")
            .map(|s| decode_text(s.as_bytes()))
            .unwrap_or_default();
        let first = spec.get::<i64>(b"St").unwrap_or(1).clamp(1, 1_000_000) as u32;
        for (offset, page) in pages
            .iter_mut()
            .enumerate()
            .skip(*start)
            .take(end.saturating_sub(*start))
        {
            let n = first + (offset - start) as u32;
            let number = match style.as_deref() {
                Some("D") => n.to_string(),
                Some("R") => roman(n, true),
                Some("r") => roman(n, false),
                Some("A") => letters(n, true),
                Some("a") => letters(n, false),
                _ => String::new(),
            };
            let label = format!("{prefix}{number}");
            page.label = (!label.is_empty()).then_some(label);
        }
    }
}

fn collect_numbers<'a>(node: &Dict<'a>, depth: usize, out: &mut Vec<(usize, Dict<'a>)>) {
    if depth > MAX_TREE_DEPTH || out.len() > MAX_PAGES {
        return;
    }
    if let Some(nums) = node.get::<Array<'a>>(b"Nums") {
        let mut it = nums.flex_iter();
        while let Some(key) = it.next::<i64>() {
            let Some(value) = it.next::<Dict<'a>>() else {
                break;
            };
            if let Ok(key) = usize::try_from(key) {
                out.push((key, value));
            }
        }
    }
    if let Some(kids) = node.get::<Array<'a>>(b"Kids") {
        for kid in kids.iter::<Dict<'a>>().take(10_000) {
            collect_numbers(&kid, depth + 1, out);
        }
    }
}

/// Count fields, signature fields, scripts and XFA without building an editable copy.
fn summarize_forms(catalog: &Dict<'_>) -> FormSummary {
    let mut s = FormSummary::default();
    let Some(acro) = catalog.get::<Dict<'_>>(b"AcroForm") else {
        return s;
    };
    let has_xfa = acro.contains_key(b"XFA");
    let mut visited = 0usize;
    let mut seen = HashSet::new();
    if let Some(fields) = acro.get::<Array<'_>>(b"Fields") {
        for f in fields.iter::<Dict<'_>>() {
            walk_field(&f, None, 0, &mut s, &mut visited, &mut seen);
        }
    }
    s.xfa = match (has_xfa, s.fields + s.signature_fields) {
        (false, _) => XfaKind::None,
        (true, 0) => XfaKind::Only,
        (true, _) => XfaKind::Hybrid,
    };
    if catalog.get::<bool>(b"NeedsRendering") == Some(true) && has_xfa {
        s.xfa = XfaKind::Only;
    }
    // Certification signature with "no changes allowed".
    if let Some(perms) = catalog.get::<Dict<'_>>(b"Perms")
        && let Some(docmdp) = perms.get::<Dict<'_>>(b"DocMDP")
        && let Some(refs) = docmdp.get::<Array<'_>>(b"Reference")
    {
        for r in refs.iter::<Dict<'_>>() {
            let p = r
                .get::<Dict<'_>>(b"TransformParams")
                .and_then(|t| t.get::<i64>(b"P"))
                .unwrap_or(2);
            if p == 1 {
                s.certified_no_changes = true;
            }
        }
    }
    s
}

fn walk_field(
    field: &Dict<'_>,
    inherited_ft: Option<String>,
    depth: usize,
    s: &mut FormSummary,
    visited: &mut usize,
    seen: &mut HashSet<ObjectIdentifier>,
) {
    *visited += 1;
    if depth > MAX_TREE_DEPTH || *visited > MAX_FIELDS {
        return;
    }
    if let Some(id) = field.obj_id()
        && !seen.insert(id)
    {
        return;
    }
    let ft = field
        .get::<Name<'_>>(b"FT")
        .map(|n| n.as_str().to_string())
        .or(inherited_ft);
    if field.contains_key(b"AA") {
        s.scripts = true;
    }
    let kids: Vec<Dict<'_>> = field
        .get::<Array<'_>>(b"Kids")
        .map(|k| k.iter::<Dict<'_>>().take(MAX_FIELDS).collect())
        .unwrap_or_default();
    // A kid that is a field has a /T; widget-only kids do not.
    let field_kids: Vec<&Dict<'_>> = kids.iter().filter(|k| k.contains_key(b"T")).collect();
    if field_kids.is_empty() {
        match ft.as_deref() {
            Some("Sig") => {
                s.signature_fields += 1;
                if field.contains_key(b"V") {
                    s.signed += 1;
                }
            }
            Some(_) => s.fields += 1,
            None => {}
        }
        for k in &kids {
            if k.contains_key(b"AA") {
                s.scripts = true;
            }
        }
        return;
    }
    for k in field_kids {
        walk_field(k, ft.clone(), depth + 1, s, visited, seen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    fn open(bytes: Vec<u8>) -> Document {
        Document::open(bytes.into(), None).expect("open")
    }

    #[test]
    fn describes_pages_outline_and_labels() {
        let doc = open(fixtures::text_pages(5));
        let info = doc.info();
        assert_eq!(info.pages.len(), 5);
        assert!((info.pages[0].width - 595.0).abs() < 1.0);
        assert_eq!(info.version, "1.7");
        assert_eq!(info.title.as_deref(), Some("Fixture"));
        assert_eq!(info.outline.len(), 3);
        assert_eq!(info.outline[0].title, "Chapter 1");
        assert!(info.outline[0].has_children);
        assert_eq!(info.outline[1].depth, 1);
        assert_eq!(info.outline[2].dest.map(|d| d.page), Some(4));
        assert_eq!(info.pages[0].label.as_deref(), Some("i"));
        assert_eq!(info.pages[2].label.as_deref(), Some("1"));
        assert_eq!(info.find_page("1"), Some(2), "labels win over numbers");
        assert_eq!(info.find_page("ii"), Some(1));
        assert_eq!(info.find_page("5"), Some(4));
        assert_eq!(info.find_page("99"), None);
        assert_eq!(info.find_page(""), None);
    }

    #[test]
    fn links_resolve_and_actions_are_inert() {
        let doc = open(fixtures::text_pages(5));
        let links = doc.links(0).expect("links");
        assert!(links.iter().any(|l| l.target
            == LinkTarget::Dest(Dest {
                page: 3,
                top: Some(0.0)
            })
            || matches!(l.target, LinkTarget::Dest(Dest { page: 3, .. }))));
        assert!(
            links
                .iter()
                .any(|l| matches!(&l.target, LinkTarget::Uri(u) if u == "https://example.com/"))
        );
        assert!(
            links
                .iter()
                .any(|l| matches!(&l.target, LinkTarget::Unsupported(w) if w.contains("program")))
        );
        let link = &links[0];
        assert!(link.rect[0] < link.rect[2] && link.rect[1] < link.rect[3]);
        assert!(doc.links(99).is_err());
    }

    #[test]
    fn rotated_page_swaps_dimensions() {
        let doc = open(fixtures::rotated_page());
        let p = &doc.info().pages[0];
        assert!(p.width > p.height, "landscape after /Rotate 90");
    }

    #[test]
    fn password_documents() {
        let bytes: Arc<[u8]> = fixtures::encrypted("secret").into();
        assert_eq!(
            Document::open(bytes.clone(), None).expect_err("needs a password"),
            OpenError::NeedsPassword { wrong: false }
        );
        let err = Document::open(bytes.clone(), Some(&Secret::new("nope"))).expect_err("wrong");
        assert_eq!(err, OpenError::NeedsPassword { wrong: true });
        assert!(!err.to_string().contains("nope"));
        let doc = Document::open(bytes, Some(&Secret::new("secret"))).expect("right password");
        assert!(doc.info().encrypted && doc.info().password_used);
    }

    #[test]
    fn forms_are_summarized() {
        let doc = open(fixtures::form());
        let f = &doc.info().forms;
        assert_eq!(f.fields, 7);
        assert_eq!(f.signature_fields, 1);
        assert_eq!(f.signed, 0);
        assert_eq!(f.xfa, XfaKind::None);
        let doc = open(fixtures::hybrid_xfa());
        assert_eq!(doc.info().forms.xfa, XfaKind::Hybrid);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for bytes in [
            b"%PDF-1.7\n".to_vec(),
            b"not a pdf".to_vec(),
            Vec::new(),
            fixtures::text_pages(2)[..200].to_vec(),
        ] {
            assert!(Document::open(bytes.into(), None).is_err());
        }
    }

    /// Truncated files, flipped bytes and absurd nesting: opening, rendering, text and
    /// links either work or fail with an error, never a panic.
    #[test]
    fn damaged_documents_never_panic() {
        let base = fixtures::text_pages(2);
        let form = fixtures::form();
        let mut cases: Vec<Vec<u8>> = Vec::new();
        for src in [&base, &form] {
            for cut in [src.len() / 3, src.len() / 2, src.len() - 40, src.len() - 5] {
                cases.push(src[..cut].to_vec());
            }
            for step in [97usize, 211, 509] {
                let mut b = src.clone();
                for i in (64..b.len()).step_by(step) {
                    b[i] ^= 0x5a;
                }
                cases.push(b);
            }
        }
        // A page whose content nests arrays and dictionaries thousands deep.
        let deep = format!(
            "%PDF-1.7\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Foo {}{}>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF",
            "[".repeat(5000),
            "]".repeat(5000)
        );
        cases.push(deep.into_bytes());
        for bytes in cases {
            let Ok(doc) = Document::open(bytes.into(), None) else {
                continue;
            };
            for page in 0..doc.page_count().min(3) {
                let _ =
                    crate::render::render_page(&doc, page, 0.5, crate::ColorMode::Original, None);
                let _ = crate::text::page_text(&doc, page);
                let _ = doc.links(page);
            }
        }
    }
}
