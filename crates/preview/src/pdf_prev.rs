//! PDF previews (change `add-pdf-preview`): the file is read once (≤ 256 MiB) and opened
//! with `fm-pdf`; pages are rendered later by the engine's PDF pool. Password-protected
//! files keep their bytes so a typed password can open them without reading again.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fm_pdf::text::PageText;
use fm_pdf::{Document, MAX_FILE_BYTES, OpenError, Secret};
use fm_vfs::{Entry, FmError, Vfs};

use crate::{MetaDoc, Preview, base_meta, text};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A fresh id for a newly opened document (render cache key).
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Extracted text and links per page, filled lazily off the UI thread and shared with
/// the view.
#[derive(Debug, Default)]
pub struct TextCache {
    pages: Mutex<HashMap<usize, Arc<PageText>>>,
    links: Mutex<HashMap<usize, Arc<Vec<fm_pdf::Link>>>>,
}

impl TextCache {
    pub fn links(&self, page: usize) -> Option<Arc<Vec<fm_pdf::Link>>> {
        self.links.lock().ok()?.get(&page).cloned()
    }

    pub fn put_links(&self, page: usize, links: Arc<Vec<fm_pdf::Link>>) {
        if let Ok(mut m) = self.links.lock() {
            m.insert(page, links);
        }
    }

    pub fn get(&self, page: usize) -> Option<Arc<PageText>> {
        self.pages.lock().ok()?.get(&page).cloned()
    }

    pub fn put(&self, page: usize, text: Arc<PageText>) {
        if let Ok(mut m) = self.pages.lock() {
            m.insert(page, text);
        }
    }

    pub fn len(&self) -> usize {
        self.pages.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What opening the document gave.
#[derive(Clone, Debug)]
pub enum PdfState {
    Ready(Arc<Document>),
    /// Needs a user password (`wrong`: the last one did not open it).
    Locked {
        wrong: bool,
    },
}

/// A PDF preview.
#[derive(Clone, Debug)]
pub struct PdfDoc {
    /// File information rows (name, size, then the document's).
    pub rows: Vec<(String, String)>,
    pub bytes: Arc<[u8]>,
    pub state: PdfState,
    pub text: Arc<TextCache>,
    /// Unique per opened document; render caches key on it.
    pub id: u64,
}

impl PartialEq for PdfDoc {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl PdfDoc {
    pub fn document(&self) -> Option<&Arc<Document>> {
        match &self.state {
            PdfState::Ready(d) => Some(d),
            PdfState::Locked { .. } => None,
        }
    }

    /// Open the same bytes with `password`.
    pub fn unlock(&self, password: &Secret) -> Result<PdfDoc, OpenError> {
        let doc = Document::open(self.bytes.clone(), Some(password))?;
        let mut rows: Vec<(String, String)> = self
            .rows
            .iter()
            .take_while(|(k, _)| k == "Name" || k == "Size")
            .cloned()
            .collect();
        rows.extend(doc.info().rows());
        Ok(PdfDoc {
            rows,
            bytes: self.bytes.clone(),
            state: PdfState::Ready(Arc::new(doc)),
            text: Arc::new(TextCache::default()),
            id: next_id(),
        })
    }
}

/// Read and open a PDF.
pub fn render(vfs: &dyn Vfs, entry: &Entry) -> Result<Preview, FmError> {
    let rows = base_meta(entry);
    let card = |hint: String| {
        Preview::Meta(MetaDoc {
            rows: base_meta(entry),
            hint: Some(hint),
            poster: None,
            video: None,
            audio: None,
        })
    };
    if entry.size.is_some_and(|s| s > MAX_FILE_BYTES) {
        return Ok(card(OpenError::TooLarge.to_string()));
    }
    let (bytes, more) = text::read_capped(vfs, entry, MAX_FILE_BYTES as usize)?;
    if more {
        return Ok(card(OpenError::TooLarge.to_string()));
    }
    let bytes: Arc<[u8]> = bytes.into();
    Ok(open_bytes(rows, bytes))
}

/// Open already-read bytes (also used after saving a filled copy).
pub fn open_bytes(mut rows: Vec<(String, String)>, bytes: Arc<[u8]>) -> Preview {
    match Document::open(bytes.clone(), None) {
        Ok(doc) => {
            rows.extend(doc.info().rows());
            Preview::Pdf(PdfDoc {
                rows,
                bytes,
                state: PdfState::Ready(Arc::new(doc)),
                text: Arc::new(TextCache::default()),
                id: next_id(),
            })
        }
        Err(OpenError::NeedsPassword { wrong }) => {
            rows.push(("Encryption".into(), "Password-protected".into()));
            Preview::Pdf(PdfDoc {
                rows,
                bytes,
                state: PdfState::Locked { wrong },
                text: Arc::new(TextCache::default()),
                id: next_id(),
            })
        }
        Err(e) => Preview::Meta(MetaDoc {
            rows,
            hint: Some(format!("{e}.")),
            poster: None,
            video: None,
            audio: None,
        }),
    }
}

/// First page of a local PDF as a thumbnail with its longest edge at `edge`. Encrypted
/// documents that need a password and pages that fail to render give `None` (a name tile).
pub fn thumbnail(vfs: &dyn Vfs, entry: &Entry, edge: u32) -> Option<crate::thumbs::Thumb> {
    if entry.size.is_some_and(|s| s > MAX_FILE_BYTES) {
        return None;
    }
    let (bytes, more) = text::read_capped(vfs, entry, MAX_FILE_BYTES as usize).ok()?;
    if more {
        return None;
    }
    let doc = Document::open(bytes.into(), None).ok()?;
    let page = doc.info().pages.first()?;
    let scale = (edge as f32 / page.width.max(page.height)).min(4.0);
    let px = fm_pdf::render::render_page(&doc, 0, scale, fm_pdf::ColorMode::Original, None).ok()?;
    let thumb = crate::thumbs::Thumb {
        width: px.width,
        height: px.height,
        rgba: px.rgba,
    };
    thumb.valid().then_some(thumb)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;
    use fm_vfs::VfsUri;

    fn put(vfs: &MemFs, path: &str, bytes: &[u8]) -> Entry {
        vfs.put_str(path, bytes).expect("file");
        vfs.stat(&VfsUri::parse(path).expect("uri")).expect("stat")
    }

    #[test]
    fn opens_documents_and_password_files() {
        let vfs = MemFs::new();
        let e = put(&vfs, "/a.pdf", &fm_pdf::fixtures::text_pages(3));
        let Preview::Pdf(doc) = render(&vfs, &e).expect("render") else {
            panic!("not a pdf preview");
        };
        assert_eq!(doc.document().map(|d| d.page_count()), Some(3));
        assert!(doc.rows.iter().any(|(k, v)| k == "Pages" && v == "3"));
        let e = put(&vfs, "/b.pdf", &fm_pdf::fixtures::encrypted("pw"));
        let Preview::Pdf(locked) = render(&vfs, &e).expect("render") else {
            panic!("not a pdf preview");
        };
        assert!(matches!(locked.state, PdfState::Locked { wrong: false }));
        assert!(locked.unlock(&Secret::new("bad")).is_err());
        let open = locked.unlock(&Secret::new("pw")).expect("unlock");
        assert!(open.document().is_some());
        assert_ne!(open.id, locked.id);
    }

    #[test]
    fn unreadable_pdf_is_an_information_card() {
        let vfs = MemFs::new();
        let e = put(&vfs, "/c.pdf", b"%PDF-1.7\nnothing here");
        match render(&vfs, &e).expect("render") {
            Preview::Meta(m) => assert!(m.hint.unwrap_or_default().contains("could not be read")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn thumbnails_of_first_pages() {
        let vfs = MemFs::new();
        let e = put(&vfs, "/d.pdf", &fm_pdf::fixtures::text_pages(2));
        let t = thumbnail(&vfs, &e, 160).expect("thumb");
        assert_eq!(t.height, 160);
        assert!(t.width < 160);
        let e = put(&vfs, "/e.pdf", &fm_pdf::fixtures::encrypted("pw"));
        assert!(thumbnail(&vfs, &e, 160).is_none(), "locked: name tile");
    }
}
