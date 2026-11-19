//! PDF documents over bytes (change `add-pdf-preview`): open and describe a document,
//! render pages to RGBA, extract text for Find and copy, fill AcroForm fields, place
//! visual signatures, sign with a PKCS#12 certificate and verify existing signatures.
//!
//! Pure and I/O-free: input is `Arc<[u8]>`, output is our own models, pixels and new
//! bytes. `hayro` reads and renders, `lopdf` builds incremental updates, RustCrypto signs
//! and verifies; none of their types leave this crate. Every public entry point runs
//! inside [`guard`] (a library panic becomes an error) and checks the caps below.
//! Documents are hostile input: no action is ever performed, no embedded file is read and
//! nothing outside the document is fetched.

pub mod cert;
pub mod doc;
pub mod forms;
pub mod render;
pub mod secret;
pub mod sign;
pub mod text;
pub mod verify;
pub mod visual;
pub mod write;

mod lo;
mod strings;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

pub use doc::{
    Dest, DocInfo, Document, FormSummary, Link, LinkTarget, OpenError, OutlineItem, PageInfo,
    Permissions, XfaKind,
};
pub use render::{ColorMode, Pixels, Tile};
pub use secret::Secret;

/// Licence of the Foxit standard fonts (from PDFium) the renderer bundles for PDFs that do
/// not embed their fonts.
pub const FOXIT_FONTS_LICENSE: &str = include_str!("../licenses/Foxit-fonts-BSD-3-Clause.txt");
/// Licence of the Adobe CMap resources the renderer bundles for CJK text.
pub const ADOBE_CMAPS_LICENSE: &str = include_str!("../licenses/Adobe-CMaps-BSD-3-Clause.txt");

/// Largest file the viewer opens (D10).
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// Largest file the editing parser opens (fill and sign).
pub const MAX_EDIT_BYTES: u64 = 64 << 20;
/// Largest page or tile rendered in one bitmap.
pub const MAX_PIXELS: u64 = 16_000_000;
/// Largest page box in points (the PDF limit).
pub const MAX_PAGE_PT: f32 = 14_400.0;
/// Pages described in [`DocInfo`]; later pages of an absurd document are not shown.
pub const MAX_PAGES: usize = 100_000;

/// Errors of this crate. `Display` never contains passwords, key bytes or document text
/// beyond what the user typed as a field value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfError {
    /// The document is larger than a cap (what, limit).
    TooLarge(String),
    /// The bytes are not a readable PDF.
    Invalid(String),
    /// A password is needed, or the given one is wrong.
    Password,
    /// Something this crate does not support (named).
    Unsupported(String),
    /// A value or parameter was refused (reason shown to the user).
    Refused(String),
    /// A certificate could not be used.
    Certificate(String),
    /// A library panicked; contained.
    Internal,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(what) => write!(f, "{what}"),
            Self::Invalid(why) => write!(f, "the PDF could not be read: {why}"),
            Self::Password => write!(f, "incorrect password"),
            Self::Unsupported(what) => write!(f, "not supported: {what}"),
            Self::Refused(why) => write!(f, "{why}"),
            Self::Certificate(why) => write!(f, "{why}"),
            Self::Internal => write!(f, "the PDF could not be processed"),
        }
    }
}

impl std::error::Error for PdfError {}

impl From<PdfError> for fm_vfs::FmError {
    fn from(e: PdfError) -> Self {
        match e {
            PdfError::Refused(why) => fm_vfs::FmError::InvalidParams(why),
            other => fm_vfs::FmError::backend("pdf", other.to_string()),
        }
    }
}

/// Run `work`, turning a panic inside the PDF libraries into [`PdfError::Internal`].
pub fn guard<T>(work: impl FnOnce() -> Result<T, PdfError>) -> Result<T, PdfError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)) {
        Ok(r) => r,
        Err(_) => Err(PdfError::Internal),
    }
}
