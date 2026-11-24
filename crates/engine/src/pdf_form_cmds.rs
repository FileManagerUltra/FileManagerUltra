//! Fill & Sign sessions and saving (design D6, D11, D12): `pdf.form.*`, `pdf.save`,
//! `pdf.save_as`, `pdf.save_overwrite`, `pdf.discard` and the unsaved-changes prompt.
//!
//! A session parses the document twice with the editing parser, off the UI thread: one
//! copy validates values on the UI thread (cheap, no I/O), the other builds previews and
//! saves on the PDF pool. What the pane draws while filling is the built preview, so
//! what you see is what will be saved. Nothing is written anywhere before the user saves.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use fm_layout::PaneId;
use fm_pdf::forms::{Field, FieldInput, FieldValue, FormDoc};
use fm_pdf::visual::{Placement, Visual};
use fm_pdf::{Document, PdfError, Secret};
use fm_preview::Preview;
use fm_vfs::{Entry, EntryKind, FmError, VfsUri};
use serde_json::Value;

use crate::fileops::{OpRequest, SharedBytes, WriteMode};
use crate::jobs::JobId;
use crate::pdf_view::{PdfMsg, Shown, push};
use crate::pdf_view_cmds::{pdf_focused, pdf_pane};
use crate::preview_cmds::Body;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FillState {
    Opening,
    Ready,
    Failed(String),
}

/// Where a save goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// Next to the original as `<name> (filled).pdf` / `(signed).pdf`.
    Copy,
    As(VfsUri),
    Overwrite,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveState {
    Idle,
    Building,
    Writing(JobId),
    Failed(String),
}

/// Asked before saving: required fields still empty, existing signatures that a visual
/// signature on a page will mark as changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavePrompt {
    pub target: SaveTarget,
    pub empty_required: Vec<String>,
    pub breaks_signatures: bool,
}

/// A digital signature to apply as the last step of a save.
#[derive(Clone, Debug)]
pub struct DigitalJob {
    pub cert: VfsUri,
    pub password: Secret,
    pub request: fm_pdf::sign::Request,
}

pub struct PdfSession {
    pub state: FillState,
    /// Fill mode is on (field overlays shown).
    pub editing: bool,
    validator: Option<FormDoc>,
    builder: Option<Arc<Mutex<FormDoc>>>,
    pub fields: Arc<Vec<Field>>,
    pub values: BTreeMap<String, FieldValue>,
    pub placements: Vec<Placement>,
    /// A visual signature made in the dialog, waiting to be placed.
    pub pending_visual: Option<Arc<Visual>>,
    pub visual_loading: bool,
    pub revision: u64,
    pub built: Option<Shown>,
    built_revision: u64,
    building: bool,
    /// Why filling is not possible (signing may still be).
    pub blocked: Option<String>,
    pub xfa_hybrid: bool,
    pub scripts: bool,
    /// The document already carries digital signatures.
    pub signed_before: bool,
    /// The last refused input, shown inline.
    pub error: Option<String>,
    pub save: SaveState,
    pub prompt: Option<SavePrompt>,
    /// The file being filled and its bytes as loaded.
    pub uri: VfsUri,
    pub original: Arc<[u8]>,
    password: Option<Secret>,
    saving_bytes: Option<Arc<[u8]>>,
    next_id: u64,
}

impl std::fmt::Debug for PdfSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfSession")
            .field("state", &self.state)
            .field("changed", &self.values.len())
            .field("placements", &self.placements.len())
            .finish_non_exhaustive()
    }
}

impl PdfSession {
    /// Unsaved values or placed signatures.
    pub fn is_dirty(&self) -> bool {
        !self.values.is_empty() || !self.placements.is_empty()
    }

    pub fn is_saving(&self) -> bool {
        matches!(self.save, SaveState::Building | SaveState::Writing(_))
    }

    pub fn value(&self, field: &str) -> Option<&FieldValue> {
        self.values.get(field).or_else(|| {
            self.fields
                .iter()
                .find(|f| f.name == field)
                .map(|f| &f.value)
        })
    }

    fn changed(&mut self) {
        self.revision += 1;
        self.error = None;
        self.prompt = None;
        if matches!(self.save, SaveState::Failed(_)) {
            self.save = SaveState::Idle;
        }
    }

    pub(crate) fn next_placement_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

/// Results of pool and lane work for sessions.
pub(crate) enum FormMsg {
    Opened {
        pane: PaneId,
        source_id: u64,
        result: Result<Box<(FormDoc, FormDoc)>, PdfError>,
    },
    Built {
        pane: PaneId,
        revision: u64,
        result: Result<Arc<Document>, String>,
    },
    SaveReady {
        pane: PaneId,
        target: SaveTarget,
        result: Result<Arc<[u8]>, String>,
    },
    Visual {
        pane: PaneId,
        result: Result<Arc<Visual>, String>,
    },
}

impl std::fmt::Debug for FormMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Opened { .. } => "Opened",
            Self::Built { .. } => "Built",
            Self::SaveReady { .. } => "SaveReady",
            Self::Visual { .. } => "Visual",
        };
        f.write_str(name)
    }
}

pub(crate) fn session_mut(s: &mut Session, pane: PaneId) -> Result<&mut PdfSession, FmError> {
    s.pdf_view_mut(pane)
        .and_then(|v| v.fill.as_mut())
        .ok_or_else(|| FmError::InvalidParams("Fill & Sign is not active for this PDF".into()))
}

pub(crate) fn ready_session(s: &mut Session, pane: PaneId) -> Result<&mut PdfSession, FmError> {
    let session = session_mut(s, pane)?;
    match &session.state {
        FillState::Ready => Ok(session),
        FillState::Opening => Err(FmError::InvalidParams(
            "The form is still being prepared.".into(),
        )),
        FillState::Failed(why) => Err(FmError::InvalidParams(why.clone())),
    }
}

fn session_ready(s: &Session) -> bool {
    let pane = s.layout.focused();
    s.pdf_view(pane)
        .and_then(|v| v.fill.as_ref())
        .is_some_and(|f| f.state == FillState::Ready)
}

fn session_dirty(s: &Session) -> bool {
    let pane = s.layout.focused();
    s.pdf_view(pane)
        .and_then(|v| v.fill.as_ref())
        .is_some_and(|f| f.state == FillState::Ready && f.is_dirty() && !f.is_saving())
}

/// `pdf.form.edit`: start (or show) Fill & Sign.
fn edit(s: &mut Session, params: &Params) -> Result<(), FmError> {
    no_extra(params)?;
    let pane = pdf_pane(s, params)?;
    if let Some(session) = s.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) {
        session.editing = !session.editing || session.is_dirty();
        return Ok(());
    }
    let doc = s
        .pdf_doc(pane)
        .cloned()
        .ok_or_else(|| FmError::InvalidParams("the PDF is not loaded".into()))?;
    let document = doc
        .document()
        .cloned()
        .ok_or_else(|| FmError::InvalidParams("unlock the PDF first".into()))?;
    let uri = s
        .preview_slot(pane)
        .and_then(|slot| slot.subject.as_ref())
        .map(|e| e.uri.clone())
        .ok_or_else(|| FmError::InvalidParams("the PDF has no file".into()))?;
    if doc.bytes.len() as u64 > fm_pdf::MAX_EDIT_BYTES {
        return Err(FmError::InvalidParams(format!(
            "Filling and signing work on PDFs up to {} MiB.",
            fm_pdf::MAX_EDIT_BYTES >> 20
        )));
    }
    let info = document.info();
    let blocked = if info.forms.xfa == fm_pdf::XfaKind::Only {
        Some(
            "This is a dynamic (XFA) form, which can only be filled in Adobe software.".to_string(),
        )
    } else if info.forms.certified_no_changes {
        Some("The author certified this document and allows no changes.".to_string())
    } else {
        info.permissions
            .filter(|p| !p.fill_forms())
            .map(|_| "The document's permissions do not allow filling in forms.".to_string())
    };
    let password = s.pdf_view(pane).and_then(|v| v.password.clone());
    let session = PdfSession {
        state: FillState::Opening,
        editing: true,
        validator: None,
        builder: None,
        fields: Arc::new(Vec::new()),
        values: BTreeMap::new(),
        placements: Vec::new(),
        pending_visual: None,
        visual_loading: false,
        revision: 0,
        built: None,
        built_revision: 0,
        building: false,
        blocked,
        xfa_hybrid: info.forms.xfa == fm_pdf::XfaKind::Hybrid,
        scripts: info.forms.scripts,
        signed_before: info.forms.signed > 0,
        error: None,
        save: SaveState::Idle,
        prompt: None,
        uri,
        original: doc.bytes.clone(),
        password: password.clone(),
        saving_bytes: None,
        next_id: 0,
    };
    let source_id = doc.id;
    if let Some(v) = s.pdf_view_mut(pane) {
        v.fill = Some(session);
    }
    let inbox = s.pdf_inbox.clone();
    let bytes = doc.bytes;
    s.pdf.run(move || {
        let result = FormDoc::open(bytes.clone(), password.as_ref())
            .and_then(|a| FormDoc::open(bytes, password.as_ref()).map(|b| Box::new((a, b))));
        push(
            &inbox,
            PdfMsg::Form(Box::new(FormMsg::Opened {
                pane,
                source_id,
                result,
            })),
        );
    });
    Ok(())
}

fn input_of(v: &Value) -> Result<FieldInput, FmError> {
    match v {
        Value::String(t) if t.len() <= fm_pdf::forms::MAX_VALUE => Ok(FieldInput::Text(t.clone())),
        Value::String(_) => Err(FmError::InvalidParams(
            "the value is longer than 32 KiB".into(),
        )),
        Value::Bool(b) => Ok(FieldInput::Bool(*b)),
        Value::Array(a) if a.len() <= 1000 => a
            .iter()
            .map(|x| {
                x.as_str()
                    .filter(|t| t.len() <= 4096)
                    .map(str::to_string)
                    .ok_or_else(|| FmError::InvalidParams("list values are short texts".into()))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(FieldInput::Many),
        _ => Err(FmError::InvalidParams(
            "value must be text, true/false or a list of texts".into(),
        )),
    }
}

/// `pdf.form.set {field, value}`.
fn set(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = crate::file_cmds::object(params)?
        .ok_or_else(|| FmError::InvalidParams("field and value are required".into()))?;
    if map
        .keys()
        .any(|k| !["pane", "field", "value"].contains(&k.as_str()))
    {
        return Err(FmError::InvalidParams("unknown parameter".into()));
    }
    let field = map
        .get("field")
        .and_then(Value::as_str)
        .filter(|f| f.len() <= 1024)
        .ok_or_else(|| FmError::InvalidParams("field must be a field name".into()))?
        .to_string();
    let input = input_of(
        map.get("value")
            .ok_or_else(|| FmError::InvalidParams("value is required".into()))?,
    )?;
    let pane = pdf_pane(s, params)?;
    let session = ready_session(s, pane)?;
    if let Some(why) = &session.blocked {
        return Err(FmError::InvalidParams(why.clone()));
    }
    if session.is_saving() {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    let validator = session
        .validator
        .as_mut()
        .ok_or_else(|| FmError::InvalidParams("the form is not ready".into()))?;
    match validator.set(&field, input) {
        Ok(()) => {
            session.values = validator.changed().clone();
            session.changed();
            s.rebuild_pdf(pane);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            session.error = Some(msg.clone());
            Err(FmError::InvalidParams(msg))
        }
    }
}

fn reset(s: &mut Session, params: &Params) -> Result<(), FmError> {
    no_extra(params)?;
    let pane = pdf_pane(s, params)?;
    let session = ready_session(s, pane)?;
    if session.is_saving() {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    if let Some(v) = session.validator.as_mut() {
        v.reset();
    }
    session.values.clear();
    session.changed();
    s.rebuild_pdf(pane);
    Ok(())
}

fn discard(s: &mut Session, params: &Params) -> Result<(), FmError> {
    no_extra(params)?;
    let pane = pdf_pane(s, params)?;
    let view = s
        .pdf_view_mut(pane)
        .ok_or_else(|| FmError::InvalidParams("no PDF".into()))?;
    if view.fill.as_ref().is_some_and(PdfSession::is_saving) {
        return Err(FmError::InvalidParams("wait for the save to finish".into()));
    }
    view.fill = None;
    Ok(())
}

fn no_extra(params: &Params) -> Result<(), FmError> {
    if let Some(m) = crate::file_cmds::object(params)?
        && m.keys().any(|k| k != "pane")
    {
        return Err(FmError::InvalidParams("unknown parameter".into()));
    }
    Ok(())
}

fn confirm_flag(params: &Params) -> Result<bool, FmError> {
    Ok(crate::file_cmds::object(params)?
        .and_then(|m| m.get("confirm"))
        .map(|v| {
            v.as_bool()
                .ok_or_else(|| FmError::InvalidParams("confirm must be true or false".into()))
        })
        .transpose()?
        .unwrap_or(false))
}

fn save_cmd(s: &mut Session, params: &Params, target: SaveTarget) -> Result<(), FmError> {
    if let Some(m) = crate::file_cmds::object(params)?
        && m.keys()
            .any(|k| !["pane", "confirm", "uri"].contains(&k.as_str()))
    {
        return Err(FmError::InvalidParams("unknown parameter".into()));
    }
    let confirm = confirm_flag(params)?;
    let pane = pdf_pane(s, params)?;
    if target == SaveTarget::Overwrite && !confirm {
        return Err(FmError::InvalidParams(
            "Replacing the original needs confirmation ({\"confirm\": true}).".into(),
        ));
    }
    s.start_save(pane, target, confirm, None)
}

fn save_as(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let uri = crate::file_cmds::object(params)?
        .and_then(|m| m.get("uri"))
        .and_then(Value::as_str)
        .filter(|u| u.len() <= 4096)
        .ok_or_else(|| FmError::InvalidParams("uri is required".into()))?;
    let uri = VfsUri::parse(uri)?;
    if uri.file_name().is_none_or(str::is_empty) {
        return Err(FmError::InvalidParams("choose a file name".into()));
    }
    save_cmd(s, params, SaveTarget::As(uri))
}

/// `<stem> (filled).pdf` or `(signed).pdf` next to `uri`.
pub(crate) fn copy_name(uri: &VfsUri, signed: bool) -> Result<VfsUri, FmError> {
    let parent = uri
        .parent()
        .ok_or_else(|| FmError::InvalidParams("the file has no folder".into()))?;
    let name = uri.file_name().unwrap_or("document.pdf");
    let stem = name
        .rsplit_once('.')
        .filter(|(s, e)| !s.is_empty() && e.eq_ignore_ascii_case("pdf"))
        .map_or(name, |(s, _)| s);
    let suffix = if signed { "signed" } else { "filled" };
    parent.join(&format!("{stem} ({suffix}).pdf"))
}

impl Session {
    /// Start a build of the session's current values for the pane to draw.
    pub(crate) fn rebuild_pdf(&mut self, pane: PaneId) {
        let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) else {
            return;
        };
        if session.building || session.built_revision == session.revision {
            return;
        }
        let Some(builder) = session.builder.clone() else {
            return;
        };
        if !session.is_dirty() {
            session.built = None;
            session.built_revision = session.revision;
            return;
        }
        session.building = true;
        let revision = session.revision;
        let values = session.values.clone();
        let placements = session.placements.clone();
        let password = session.password.clone();
        let inbox = self.pdf_inbox.clone();
        self.pdf.run(move || {
            let result = build_bytes(&builder, &values, &placements).and_then(|bytes| {
                Document::open(bytes.into(), password.as_ref())
                    .map(Arc::new)
                    .map_err(|e| e.to_string())
            });
            push(
                &inbox,
                PdfMsg::Form(Box::new(FormMsg::Built {
                    pane,
                    revision,
                    result,
                })),
            );
        });
    }

    /// Validate, then build (and sign) the final bytes off the UI thread and write them
    /// through a file job.
    pub(crate) fn start_save(
        &mut self,
        pane: PaneId,
        target: SaveTarget,
        confirmed: bool,
        digital: Option<DigitalJob>,
    ) -> Result<(), FmError> {
        let session = ready_session(self, pane)?;
        if session.is_saving() {
            return Err(FmError::InvalidParams("a save is already running".into()));
        }
        if !session.is_dirty() && digital.is_none() {
            return Err(FmError::InvalidParams(
                "There are no changes to save.".into(),
            ));
        }
        if !confirmed {
            let empty_required = session
                .validator
                .as_ref()
                .map(FormDoc::empty_required)
                .unwrap_or_default();
            let breaks_signatures =
                session.signed_before && session.placements.iter().any(|p| p.field.is_none());
            if !empty_required.is_empty() || breaks_signatures {
                session.prompt = Some(SavePrompt {
                    target,
                    empty_required,
                    breaks_signatures,
                });
                return Ok(());
            }
        }
        session.prompt = None;
        session.save = SaveState::Building;
        let builder = session
            .builder
            .clone()
            .ok_or_else(|| FmError::InvalidParams("the form is not ready".into()))?;
        let values = session.values.clone();
        let placements = session.placements.clone();
        let password = session.password.clone();
        let inbox = self.pdf_inbox.clone();
        match digital {
            None => {
                self.pdf.run(move || {
                    let result = build_bytes(&builder, &values, &placements).map(Arc::from);
                    push(
                        &inbox,
                        PdfMsg::Form(Box::new(FormMsg::SaveReady {
                            pane,
                            target,
                            result,
                        })),
                    );
                });
            }
            Some(job) => {
                // The certificate may be on any backend: read it on its lane, then sign
                // there too; the key lives only inside this closure.
                let vfs = self.backends.require(&job.cert)?;
                let lane = crate::backends::lane_for(job.cert.scheme());
                self.jobs.submit(lane, move |_ctx| {
                    let result = (|| {
                        let cert = read_capped(&*vfs, &job.cert, fm_pdf::cert::MAX_P12_BYTES)?;
                        let bytes = build_bytes(&builder, &values, &placements)?;
                        let signer =
                            fm_pdf::cert::load_pkcs12(&cert, &job.password, job.request.time)
                                .map_err(|e| e.to_string())?;
                        let signed =
                            fm_pdf::sign::sign(&bytes, password.as_ref(), &signer, &job.request)
                                .map_err(|e| e.to_string())?;
                        drop(signer);
                        Ok::<_, String>(Arc::<[u8]>::from(signed))
                    })();
                    push(
                        &inbox,
                        PdfMsg::Form(Box::new(FormMsg::SaveReady {
                            pane,
                            target,
                            result,
                        })),
                    );
                    Ok(())
                });
            }
        }
        Ok(())
    }

    pub(crate) fn apply_form_msg(&mut self, msg: FormMsg) {
        match msg {
            FormMsg::Opened {
                pane,
                source_id,
                result,
            } => {
                let Some(view) = self.pdf_view_mut(pane) else {
                    return;
                };
                if view.source_id != source_id {
                    return;
                }
                let Some(session) = view.fill.as_mut() else {
                    return;
                };
                match result.map(|b| *b) {
                    Ok((validator, builder)) => {
                        if session.blocked.is_none() {
                            session.blocked = validator.fill_blocked();
                        }
                        session.fields = Arc::new(validator.fields().to_vec());
                        session.validator = Some(validator);
                        session.builder = Some(Arc::new(Mutex::new(builder)));
                        session.state = FillState::Ready;
                    }
                    Err(e) => {
                        session.state = FillState::Failed(match e {
                            PdfError::Password => {
                                "Enter the document's password to fill it.".to_string()
                            }
                            other => other.to_string(),
                        });
                    }
                }
            }
            FormMsg::Built {
                pane,
                revision,
                result,
            } => {
                let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) else {
                    return;
                };
                session.building = false;
                match result {
                    Ok(doc) => {
                        session.built = Some(Shown {
                            id: fm_preview::pdf_prev::next_id(),
                            doc,
                            text: Arc::new(fm_preview::pdf_prev::TextCache::default()),
                        });
                        session.built_revision = revision;
                    }
                    Err(e) => session.error = Some(e),
                }
                if session.revision != revision {
                    self.rebuild_pdf(pane);
                }
            }
            FormMsg::SaveReady {
                pane,
                target,
                result,
            } => self.write_saved(pane, target, result),
            FormMsg::Visual { pane, result } => {
                let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) else {
                    return;
                };
                session.visual_loading = false;
                match result {
                    Ok(v) => session.pending_visual = Some(v),
                    Err(e) => session.error = Some(e),
                }
            }
        }
    }

    fn write_saved(&mut self, pane: PaneId, target: SaveTarget, result: Result<Arc<[u8]>, String>) {
        let signed = self
            .pdf_view(pane)
            .and_then(|v| v.fill.as_ref())
            .is_some_and(|f| !f.placements.is_empty());
        let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) else {
            return;
        };
        let bytes = match result {
            Ok(b) => b,
            Err(e) => {
                session.save = SaveState::Failed(e.clone());
                self.notice = Some(format!("The PDF was not saved: {e}"));
                self.close_save_failed();
                return;
            }
        };
        let digital = bytes
            .get(session.original.len()..)
            .is_some_and(|tail| fm_pdf::doc::find(tail, b"/ETSI.CAdES.detached").is_some());
        let (dest, mode) = match &target {
            SaveTarget::Copy => match copy_name(&session.uri, signed || digital) {
                Ok(d) => (d, WriteMode::Numbered),
                Err(e) => {
                    session.save = SaveState::Failed(e.to_string());
                    return;
                }
            },
            SaveTarget::As(uri) => (uri.clone(), WriteMode::Ask),
            SaveTarget::Overwrite => (
                session.uri.clone(),
                WriteMode::Replace {
                    original: SharedBytes(session.original.clone()),
                },
            ),
        };
        session.saving_bytes = Some(bytes.clone());
        match self.submit_op(OpRequest::WriteBytes {
            dest,
            bytes: SharedBytes(bytes),
            mode,
        }) {
            Ok(id) => {
                if let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) {
                    session.save = SaveState::Writing(id);
                }
            }
            Err(e) => {
                if let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) {
                    session.save = SaveState::Failed(e.to_string());
                }
                self.notice = Some(format!("The PDF was not saved: {e}"));
                self.close_save_failed();
            }
        }
    }

    /// A file job finished: if it was a PDF save, show the saved file or keep the changes.
    pub(crate) fn pdf_save_finished(
        &mut self,
        id: JobId,
        state: &crate::fileops::OpState,
        created: &[VfsUri],
    ) {
        let pane = self.previews.slots.iter().find_map(|(p, slot)| {
            slot.pdf
                .as_ref()
                .and_then(|v| v.fill.as_ref())
                .filter(|f| f.save == SaveState::Writing(id))
                .map(|_| *p)
        });
        let Some(pane) = pane else {
            return;
        };
        let Some(session) = self.pdf_view_mut(pane).and_then(|v| v.fill.as_mut()) else {
            return;
        };
        match state {
            crate::fileops::OpState::Finished if let Some(first) = created.first() => {
                let saved = first.clone();
                let bytes = session
                    .saving_bytes
                    .take()
                    .unwrap_or_else(|| Arc::from(Vec::new()));
                let xfa = session.xfa_hybrid;
                if let Some(v) = self.pdf_view_mut(pane) {
                    v.fill = None;
                }
                let mut entry = Entry::new(saved.clone(), EntryKind::File);
                entry.size = Some(bytes.len() as u64);
                let rows = fm_preview::base_meta(&entry);
                let preview = fm_preview::pdf_prev::open_bytes(rows, bytes);
                self.preview_show_saved(pane, entry, preview);
                let name = saved.file_name().unwrap_or("the file").to_string();
                let mut info = format!("Saved {name}.");
                if xfa {
                    info.push_str(
                        " The form's XFA part was removed so every viewer shows the filled values.",
                    );
                }
                self.info = Some((info, std::time::Instant::now()));
                self.close_save_finished(&crate::fileops::OpState::Finished);
            }
            crate::fileops::OpState::Finished => {
                // Skipped at the conflict prompt: nothing written, changes kept.
                session.save = SaveState::Idle;
                session.saving_bytes = None;
                self.close_save_failed();
            }
            crate::fileops::OpState::Cancelled => {
                session.save = SaveState::Idle;
                session.saving_bytes = None;
                self.close_save_failed();
            }
            crate::fileops::OpState::Failed(e) => {
                session.save = SaveState::Failed(e.clone());
                session.saving_bytes = None;
                self.notice = Some(format!("The PDF was not saved: {e}"));
                self.close_save_failed();
            }
            crate::fileops::OpState::Running => {}
        }
    }

    /// Show the saved file in `pane` until its source cursor moves on.
    fn preview_show_saved(&mut self, pane: PaneId, entry: Entry, preview: Preview) {
        if let Some(slot) = self.previews.slots.get_mut(&pane) {
            slot.show_saved(entry, Body::Static(Arc::new(preview)));
        }
        self.sync_pdf_views();
    }

    /// Panes holding unsaved PDF changes, with their file names.
    pub fn pdf_unsaved_panes(&self) -> Vec<(PaneId, String)> {
        self.previews
            .slots
            .iter()
            .filter_map(|(p, slot)| {
                let f = slot.pdf.as_ref()?.fill.as_ref()?;
                f.is_dirty()
                    .then(|| (*p, f.uri.file_name().unwrap_or("PDF").to_string()))
            })
            .collect()
    }
}

/// Apply `values` and `placements` to the builder and assemble the bytes.
fn build_bytes(
    builder: &Arc<Mutex<FormDoc>>,
    values: &BTreeMap<String, FieldValue>,
    placements: &[Placement],
) -> Result<Vec<u8>, String> {
    let mut form = builder.lock().unwrap_or_else(|p| p.into_inner());
    form.apply_values(values);
    form.build(placements).map_err(|e| e.to_string())
}

pub(crate) fn read_capped(
    vfs: &dyn fm_vfs::Vfs,
    uri: &VfsUri,
    cap: usize,
) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut r = vfs.open_read(uri).map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.by_ref()
        .take(cap as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() > cap {
        return Err(format!("the file is larger than {} KiB", cap >> 10));
    }
    Ok(buf)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "pdf.form.edit",
            label: "Fill & Sign",
            menu: &["Preview", "PDF", "Fill & Sign"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: pdf_focused,
            run: edit,
        },
        CommandSpec {
            id: "pdf.form.set",
            label: "Set Form Field",
            menu: &["Preview", "PDF", "Set Field"],
            shortcut: None,
            params: "{field: string (full name), value: string | bool | [string], pane?: number}",
            enabled: session_ready,
            run: set,
        },
        CommandSpec {
            id: "pdf.form.reset",
            label: "Reset Form",
            menu: &["Preview", "PDF", "Reset Form"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: session_ready,
            run: reset,
        },
        CommandSpec {
            id: "pdf.save",
            label: "Save Filled Copy",
            menu: &["Preview", "PDF", "Save Copy"],
            shortcut: None,
            params: "{confirm?: bool, pane?: number} (confirm: save despite empty required fields)",
            enabled: session_dirty,
            run: |s, p| save_cmd(s, p, SaveTarget::Copy),
        },
        CommandSpec {
            id: "pdf.save_as",
            label: "Save PDF As",
            menu: &["Preview", "PDF", "Save As"],
            shortcut: None,
            params: "{uri: string, confirm?: bool, pane?: number}",
            enabled: session_dirty,
            run: save_as,
        },
        CommandSpec {
            id: "pdf.save_overwrite",
            label: "Replace Original PDF",
            menu: &["Preview", "PDF", "Replace Original"],
            shortcut: None,
            params: "{confirm: true, pane?: number} (refused when the file changed on disk)",
            enabled: session_dirty,
            run: |s, p| save_cmd(s, p, SaveTarget::Overwrite),
        },
        CommandSpec {
            id: "pdf.discard",
            label: "Discard PDF Changes",
            menu: &["Preview", "PDF", "Discard Changes"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: |s| {
                let pane = s.layout.focused();
                s.pdf_view(pane).is_some_and(|v| v.fill.is_some())
            },
            run: discard,
        },
    ]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::pdf_view_cmds::tests::pdf_session;
    use serde_json::json;
    use std::time::{Duration, Instant};

    pub(crate) fn wait(s: &mut Session, mut done: impl FnMut(&Session) -> bool) {
        let start = Instant::now();
        while !done(s) {
            assert!(start.elapsed() < Duration::from_secs(30), "timed out");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A session filling `bytes`, saved as `form.pdf` in a temp dir.
    pub(crate) fn filling(bytes: Vec<u8>) -> (Session, PaneId, tempfile::TempDir) {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("form.pdf");
        std::fs::write(&path, &bytes).expect("write");
        let (mut s, pane) = pdf_session(bytes);
        let uri = VfsUri::parse(&path.to_string_lossy()).expect("uri");
        if let Some(slot) = s.previews.slots.get_mut(&pane) {
            slot.subject = Some(Entry::new(uri, EntryKind::File));
        }
        s.dispatch("pdf.form.edit", &json!({"pane": pane}))
            .expect("edit");
        wait(&mut s, |s| {
            s.pdf_view(pane)
                .and_then(|v| v.fill.as_ref())
                .is_some_and(|f| f.state != FillState::Opening)
        });
        (s, pane, tmp)
    }

    #[test]
    fn fill_save_copy_keeps_the_original() {
        let original = fm_pdf::fixtures::form();
        let (mut s, pane, tmp) = filling(original.clone());
        let p = |v: Value| {
            let mut v = v;
            v["pane"] = json!(pane);
            v
        };
        s.dispatch(
            "pdf.form.set",
            &p(json!({"field": "name", "value": "Jane Doe"})),
        )
        .expect("text");
        s.dispatch("pdf.form.set", &p(json!({"field": "agree", "value": true})))
            .expect("checkbox");
        assert!(
            s.pdf_view(pane).is_some_and(|v| v.holds()),
            "unsaved changes hold"
        );
        // Refusals change nothing.
        for bad in [
            json!({"field": "nope", "value": "x"}),
            json!({"field": "country", "value": "France"}),
            json!({"field": "name", "value": "x".repeat(40_000)}),
            json!({"field": "name"}),
            json!({"field": 3, "value": "x"}),
        ] {
            assert!(
                s.dispatch("pdf.form.set", &p(bad.clone())).is_err(),
                "{bad}"
            );
        }
        assert_eq!(
            s.pdf_view(pane)
                .and_then(|v| v.fill.as_ref())
                .map(|f| f.values.len()),
            Some(2)
        );
        // The pane draws the filled preview once built.
        wait(&mut s, |s| {
            s.pdf_view(pane)
                .and_then(|v| v.fill.as_ref())
                .is_some_and(|f| f.built.is_some())
        });
        s.dispatch("pdf.save", &p(json!({}))).expect("save");
        wait(&mut s, |s| {
            s.pdf_view(pane).is_some_and(|v| v.fill.is_none())
        });
        let saved = tmp.path().join("form (filled).pdf");
        let bytes = std::fs::read(&saved).expect("saved copy");
        assert!(bytes.starts_with(&original));
        assert_eq!(
            std::fs::read(tmp.path().join("form.pdf")).expect("orig"),
            original,
            "byte-for-byte unchanged"
        );
        let doc = Document::open(bytes.into(), None).expect("reopen");
        let t = fm_pdf::text::page_text(&doc, 0).expect("text");
        assert!(t.text.contains("Jane Doe"));
        // The pane now shows the saved file.
        assert_eq!(
            s.preview_slot(pane)
                .and_then(|sl| sl.subject.as_ref())
                .and_then(|e| e.uri.file_name().map(str::to_string)),
            Some("form (filled).pdf".to_string())
        );
        // A second save of the same original gets the next free name.
        let (mut s, pane, _keep) = (s, pane, tmp);
        let _ = (&mut s, pane);
    }

    #[test]
    fn required_fields_are_asked_about_and_discard_resumes() {
        let (mut s, pane, _tmp) = filling(fm_pdf::fixtures::form());
        s.dispatch(
            "pdf.form.set",
            &json!({"field": "agree", "value": true, "pane": pane}),
        )
        .expect("set");
        s.dispatch("pdf.save", &json!({"pane": pane})).expect("ask");
        let prompt = s
            .pdf_view(pane)
            .and_then(|v| v.fill.as_ref())
            .and_then(|f| f.prompt.clone())
            .expect("prompt");
        assert_eq!(prompt.empty_required, ["name"]);
        assert!(
            s.dispatch("pdf.save_overwrite", &json!({"pane": pane}))
                .is_err(),
            "overwrite needs confirmation"
        );
        s.dispatch("pdf.discard", &json!({"pane": pane}))
            .expect("discard");
        assert!(s.pdf_view(pane).is_some_and(|v| !v.holds()));
    }

    #[test]
    fn overwrite_is_refused_after_an_outside_change() {
        let (mut s, pane, tmp) = filling(fm_pdf::fixtures::form());
        s.dispatch(
            "pdf.form.set",
            &json!({"field": "name", "value": "X", "pane": pane}),
        )
        .expect("set");
        std::fs::write(tmp.path().join("form.pdf"), b"changed by someone else").expect("w");
        s.dispatch(
            "pdf.save_overwrite",
            &json!({"confirm": true, "pane": pane}),
        )
        .expect("start");
        wait(&mut s, |s| {
            s.pdf_view(pane)
                .and_then(|v| v.fill.as_ref())
                .is_some_and(|f| matches!(f.save, SaveState::Failed(_)))
        });
        assert_eq!(
            std::fs::read(tmp.path().join("form.pdf")).expect("r"),
            b"changed by someone else"
        );
        assert!(s.pdf_view(pane).is_some_and(|v| v.holds()), "changes kept");
    }

    #[test]
    fn quit_asks_and_cancel_keeps_everything() {
        let (mut s, pane, _tmp) = filling(fm_pdf::fixtures::form());
        s.dispatch(
            "pdf.form.set",
            &json!({"field": "name", "value": "X", "pane": pane}),
        )
        .expect("set");
        s.request_quit();
        assert!(s.running, "not quit yet");
        assert_eq!(
            s.unsaved_prompt.as_ref().map(|p| p.pdfs.len()),
            Some(1),
            "the one prompt lists the PDF"
        );
        s.dispatch("workspace.prompt", &json!({"choice": "cancel"}))
            .expect("cancel");
        assert!(s.running && s.pdf_view(pane).is_some_and(|v| v.holds()));
        s.request_quit();
        assert!(
            s.dispatch("workspace.prompt", &json!({"choice": "maybe"}))
                .is_err()
        );
        s.dispatch("workspace.prompt", &json!({"choice": "discard"}))
            .expect("discard");
        assert!(!s.running, "quit after discarding");
    }

    #[test]
    fn copy_names() {
        let uri = VfsUri::parse("/docs/contract.pdf").expect("uri");
        assert_eq!(
            copy_name(&uri, true).expect("name").file_name(),
            Some("contract (signed).pdf")
        );
        let uri = VfsUri::parse("/docs/scan").expect("uri");
        assert_eq!(
            copy_name(&uri, false).expect("name").file_name(),
            Some("scan (filled).pdf")
        );
    }
}
