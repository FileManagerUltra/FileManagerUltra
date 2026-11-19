//! Per-pane PDF view state (change `add-pdf-preview`): fit and colour modes, the current
//! page and navigation requests, the unlocked document, text selection, and the fill &
//! sign session. The view follows the slot's `Preview::Pdf`; a session with unsaved
//! changes holds the pane on its document (design D12).

use std::sync::{Arc, Mutex};

use fm_layout::PaneId;
use fm_pdf::{ColorMode, Document, Secret};
use fm_preview::Preview;
use fm_preview::pdf_prev::{PdfDoc, TextCache};
use serde::{Deserialize, Serialize};

use crate::pdf_form_cmds::PdfSession;
use crate::preview_cmds::Body;
use crate::session::Session;

/// How pages are sized before the pane's zoom multiplies them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfFit {
    #[default]
    Width,
    Page,
    Actual,
}

impl PdfFit {
    pub const ALL: [Self; 3] = [Self::Width, Self::Page, Self::Actual];

    pub fn id(self) -> &'static str {
        match self {
            Self::Width => "width",
            Self::Page => "page",
            Self::Actual => "actual",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Width => "Fit width",
            Self::Page => "Fit page",
            Self::Actual => "100%",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == id)
    }
}

/// Page colours for low-vision reading (design D4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PdfColors {
    #[default]
    Original,
    Dark,
    HighContrast,
}

impl PdfColors {
    pub const ALL: [Self; 3] = [Self::Original, Self::Dark, Self::HighContrast];

    pub fn id(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Dark => "dark",
            Self::HighContrast => "high_contrast",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Dark => "Dark pages",
            Self::HighContrast => "High contrast",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }
}

/// A navigation request the view carries out once (`seq` changes per request).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Goto {
    pub page: usize,
    /// Page-space distance from the page top, when the destination names one.
    pub top: Option<f32>,
    pub seq: u64,
}

/// Selected text: from `(page, byte)` to `(page, byte)` in extracted page text, ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PdfSelection {
    pub start: (usize, usize),
    pub end: (usize, usize),
}

/// The document a pane currently draws.
#[derive(Clone, Debug)]
pub struct Shown {
    pub id: u64,
    pub doc: Arc<Document>,
    pub text: Arc<TextCache>,
}

#[derive(Debug)]
pub struct PdfView {
    /// Id of the `Preview::Pdf` this view belongs to.
    pub source_id: u64,
    pub fit: PdfFit,
    pub colors: PdfColors,
    /// Text and background colours for High contrast (from the theme).
    pub contrast: ([u8; 3], [u8; 3]),
    /// Topmost visible page.
    pub page: usize,
    pub goto: Option<Goto>,
    pub visible: Vec<usize>,
    pub outline_open: bool,
    pub signatures_open: bool,
    /// Held only while this document is shown; never stored or logged.
    pub password: Option<Secret>,
    pub unlocking: bool,
    pub password_error: Option<String>,
    pub selection: Option<PdfSelection>,
    /// Copy asked for before every page's text was extracted.
    pub copy_pending: bool,
    pub fill: Option<PdfSession>,
    goto_seq: u64,
}

impl PdfView {
    pub(crate) fn new(source_id: u64, fit: PdfFit, colors: PdfColors) -> Self {
        Self {
            source_id,
            fit,
            colors,
            contrast: ([255, 255, 255], [0, 0, 0]),
            page: 0,
            goto: None,
            visible: Vec::new(),
            outline_open: false,
            signatures_open: false,
            password: None,
            unlocking: false,
            password_error: None,
            selection: None,
            copy_pending: false,
            fill: None,
            goto_seq: 0,
        }
    }

    pub(crate) fn go(&mut self, page: usize, top: Option<f32>) {
        self.goto_seq += 1;
        self.page = page;
        self.goto = Some(Goto {
            page,
            top,
            seq: self.goto_seq,
        });
    }

    /// Unsaved form values or placed signatures keep the pane on this document.
    pub fn holds(&self) -> bool {
        self.fill.as_ref().is_some_and(PdfSession::is_dirty)
    }

    pub fn color_mode(&self) -> ColorMode {
        match self.colors {
            PdfColors::Original => ColorMode::Original,
            PdfColors::Dark => ColorMode::Dark,
            PdfColors::HighContrast => ColorMode::HighContrast {
                fg: self.contrast.0,
                bg: self.contrast.1,
            },
        }
    }
}

/// Results from the PDF pool, applied in `poll`.
#[derive(Debug)]
pub(crate) enum PdfMsg {
    Unlocked {
        pane: PaneId,
        source_id: u64,
        result: Result<PdfDoc, fm_pdf::OpenError>,
    },
    Form(Box<crate::pdf_form_cmds::FormMsg>),
    Saved(Result<Vec<crate::pdf_sign_cmds::SavedSignature>, String>),
}

pub(crate) type PdfInbox = Arc<Mutex<Vec<PdfMsg>>>;

pub(crate) fn push(inbox: &PdfInbox, msg: PdfMsg) {
    if let Ok(mut g) = inbox.lock() {
        g.push(msg);
    }
}

impl Session {
    /// The `Preview::Pdf` a Preview pane shows, if any.
    pub fn pdf_doc(&self, pane: PaneId) -> Option<&PdfDoc> {
        match &self.preview_slot(pane)?.body {
            Body::Static(p) => match &**p {
                Preview::Pdf(d) => Some(d),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn pdf_view(&self, pane: PaneId) -> Option<&PdfView> {
        self.preview_slot(pane)?.pdf.as_ref()
    }

    pub(crate) fn pdf_view_mut(&mut self, pane: PaneId) -> Option<&mut PdfView> {
        self.previews.slots.get_mut(&pane)?.pdf.as_mut()
    }

    /// The document pane `pane` draws: the filled preview while filling, else the
    /// unlocked or loaded document.
    pub fn pdf_shown(&self, pane: PaneId) -> Option<Shown> {
        let view = self.pdf_view(pane)?;
        if let Some(built) = view.fill.as_ref().and_then(|f| f.built.clone()) {
            return Some(built);
        }
        let _ = view;
        let doc = self.pdf_doc(pane)?;
        Some(Shown {
            id: doc.id,
            doc: doc.document()?.clone(),
            text: doc.text.clone(),
        })
    }

    /// Give every PDF slot its view (new document: fresh view; no PDF: none, unless it
    /// holds unsaved changes) and start verification of signed documents.
    pub(crate) fn sync_pdf_views(&mut self) -> bool {
        let fit = self.prefs.pdf_fit;
        let colors = self.prefs.pdf_colors;
        let mut changed = false;
        let mut verify: Vec<(u64, Arc<Document>)> = Vec::new();
        for slot in self.previews.slots.values_mut() {
            let doc = match &slot.body {
                Body::Static(p) => match &**p {
                    Preview::Pdf(d) => Some(d),
                    _ => None,
                },
                _ => None,
            };
            match (doc, &mut slot.pdf) {
                (Some(d), Some(v)) if v.source_id == d.id => {}
                (_, Some(v)) if v.holds() => {}
                (Some(d), view) => {
                    let (fit, colors) = view.as_ref().map_or((fit, colors), |v| (v.fit, v.colors));
                    *view = Some(PdfView::new(d.id, fit, colors));
                    changed = true;
                }
                (None, view @ Some(_)) => {
                    *view = None;
                    changed = true;
                }
                (None, None) => {}
            }
            if slot.pdf.is_some()
                && let Some(d) = doc
                && let Some(document) = d.document()
                && document.info().forms.signed > 0
            {
                verify.push((d.id, document.clone()));
            }
        }
        for (id, doc) in verify {
            self.pdf.want_verify(id, &doc);
        }
        changed
    }

    /// Apply pool results and finish pending copies. Called from `poll`.
    pub(crate) fn pump_pdf(&mut self) -> bool {
        let mut changed = self.pdf.supervise();
        changed |= self.sync_pdf_views();
        let msgs: Vec<PdfMsg> = match self.pdf_inbox.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => Vec::new(),
        };
        for msg in msgs {
            changed = true;
            match msg {
                PdfMsg::Unlocked {
                    pane,
                    source_id,
                    result,
                } => {
                    let Some(view) = self.pdf_view_mut(pane) else {
                        continue;
                    };
                    if view.source_id != source_id {
                        continue;
                    }
                    view.unlocking = false;
                    match result {
                        Ok(doc) => {
                            view.password_error = None;
                            crate::pdf_view_cmds::replace_body(self, pane, doc);
                        }
                        Err(e) => {
                            view.password = None;
                            view.password_error = Some(match e {
                                fm_pdf::OpenError::NeedsPassword { .. } => {
                                    "Incorrect password. Try again.".to_string()
                                }
                                other => other.to_string(),
                            });
                        }
                    }
                }
                PdfMsg::Form(m) => self.apply_form_msg(*m),
                PdfMsg::Saved(result) => {
                    self.pdf_saved_loading = false;
                    match result {
                        Ok(list) => self.pdf_saved = list,
                        Err(e) => {
                            self.notice = Some(format!("Remembered signatures: {e}"));
                        }
                    }
                }
            }
        }
        changed |= self.finish_pdf_copies();
        changed
    }

    fn finish_pdf_copies(&mut self) -> bool {
        let panes: Vec<PaneId> = self
            .previews
            .slots
            .iter()
            .filter(|(_, s)| s.pdf.as_ref().is_some_and(|v| v.copy_pending))
            .map(|(p, _)| *p)
            .collect();
        let mut changed = false;
        for pane in panes {
            let Some(shown) = self.pdf_shown(pane) else {
                continue;
            };
            let n = shown.doc.page_count();
            if (0..n).all(|p| self.pdf.text_done(shown.id, &shown.text, p)) {
                let selection = self.pdf_view(pane).and_then(|v| v.selection);
                let text = crate::pdf_view_cmds::selected_text(&shown, selection);
                self.clipboard_out = Some(text);
                if let Some(v) = self.pdf_view_mut(pane) {
                    v.copy_pending = false;
                }
                changed = true;
            } else if !self.pdf.text_pending(shown.id) {
                for p in 0..n {
                    self.pdf.want_text(shown.id, &shown.doc, &shown.text, p);
                }
            }
        }
        changed
    }
}
