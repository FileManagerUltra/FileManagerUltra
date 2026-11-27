//! The session side of the text editor (capability `text-editor`): which Preview panes
//! are in Edit mode, their load/save/parse jobs, autosave triggers, the leave warning,
//! conflicts with the file on disk, and following the source Files pane while clean.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fm_editor::{Buffer, Lang, Pos, Syntax, SyntaxError};
use fm_layout::{EditViewState, PaneId, PaneKind};
use fm_vfs::{Entry, FmError, Scheme, VfsUri};

use crate::editor_doc::{EditorDoc, Pending, Phase};
use crate::editor_io::{self, EdMsg, Inbox, SaveFail, SaveJob};
use crate::session::Session;

/// Syntax parses running at once, over all editors.
const MAX_PARSERS: usize = 2;
/// Text scanned for the line-ending style and the language's first line.
const DETECT_CHARS: usize = 1 << 20;

#[derive(Default)]
pub struct EditorHub {
    pub docs: HashMap<PaneId, EditorDoc>,
    pub(crate) inbox: Inbox,
    next_token: u64,
    /// The pane focused at the last poll: a move away from an edited pane is a focus loss.
    last_focus: Option<PaneId>,
    parsing: Arc<AtomicUsize>,
}

impl std::fmt::Debug for EditorHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EditorHub")
            .field("docs", &self.docs.len())
            .finish()
    }
}

impl EditorHub {
    pub fn doc(&self, pane: PaneId) -> Option<&EditorDoc> {
        self.docs.get(&pane)
    }

    pub fn is_busy(&self) -> bool {
        self.docs
            .values()
            .any(|d| d.phase != Phase::Ready || d.parse_inflight.is_some() || d.check_inflight)
    }

    fn token(&mut self) -> u64 {
        self.next_token += 1;
        self.next_token
    }

    /// Panes with unsaved edits and their file names.
    pub fn unsaved(&self) -> Vec<(PaneId, String)> {
        let mut out: Vec<(PaneId, String)> = self
            .docs
            .iter()
            .filter(|(_, d)| d.dirty())
            .map(|(p, d)| (*p, d.name.clone()))
            .collect();
        out.sort();
        out
    }
}

/// File names that are never text: the editor does not offer Edit mode for them.
/// Whether a blocked load goes to the default application instead of the blocked card:
/// only a double-clicked file (`activate`) that turned out binary. Too large or not a file
/// keep the card, which explains why.
pub(crate) fn hands_off_to_app(activated: bool, why: &editor_io::Blocked) -> bool {
    activated && *why == editor_io::Blocked::Binary
}

pub(crate) fn binary_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    matches!(
        ext,
        "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "bmp"
            | "ico"
            | "tif"
            | "tiff"
            | "heic"
            | "heif"
            | "avif"
            | "psd"
            | "raw"
            | "cr2"
            | "nef"
            | "dng"
            | "mp3"
            | "flac"
            | "wav"
            | "ogg"
            | "oga"
            | "m4a"
            | "aac"
            | "opus"
            | "wma"
            | "mp4"
            | "m4v"
            | "mkv"
            | "avi"
            | "mov"
            | "webm"
            | "wmv"
            | "mpg"
            | "mpeg"
            | "pdf"
            | "zip"
            | "7z"
            | "rar"
            | "gz"
            | "tgz"
            | "xz"
            | "bz2"
            | "zst"
            | "tar"
            | "iso"
            | "dmg"
            | "img"
            | "exe"
            | "dll"
            | "so"
            | "dylib"
            | "bin"
            | "class"
            | "jar"
            | "o"
            | "a"
            | "lib"
            | "wasm"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "odt"
            | "rtf"
            | "ods"
            | "odp"
            | "ttf"
            | "otf"
            | "woff"
            | "woff2"
            | "sqlite"
            | "db"
            | "pyc"
    )
}

/// Whether `entry` may be opened in Edit mode; the error says why not.
pub fn check_editable(entry: &Entry) -> Result<(), FmError> {
    if entry.is_dir_like() {
        return Err(FmError::InvalidParams("folders cannot be edited".into()));
    }
    if binary_name(&entry.name) {
        return Err(FmError::InvalidParams(
            "this type of file cannot be edited; open it in an external application".into(),
        ));
    }
    if entry
        .size
        .is_some_and(|s| s > crate::editor_doc::MAX_EDIT_BYTES)
    {
        return Err(FmError::InvalidParams(
            "files over 1 GiB are not opened for editing; use Open in External Editor".into(),
        ));
    }
    Ok(())
}

fn is_archive_member(uri: &VfsUri) -> bool {
    uri.scheme() == Scheme::Archive || uri.archive_handle().is_some()
}

impl Session {
    pub fn editor(&self, pane: PaneId) -> Option<&EditorDoc> {
        self.editors.docs.get(&pane)
    }

    pub fn editor_mut(&mut self, pane: PaneId) -> Option<&mut EditorDoc> {
        self.editors.docs.get_mut(&pane)
    }

    /// The single file a Preview pane shows, if it shows exactly one.
    pub(crate) fn preview_subject(&self, pane: PaneId) -> Option<Entry> {
        let slot = self.previews.slot(pane)?;
        match slot.subjects.as_slice() {
            [one] => Some(one.clone()),
            [] => slot.subject.clone(),
            _ => None,
        }
    }

    /// Opens `uri` in Edit mode in Preview pane `pane`, replacing whatever it edited.
    pub(crate) fn editor_load(
        &mut self,
        pane: PaneId,
        uri: VfsUri,
        caret: Option<Pos>,
        top: usize,
    ) -> Result<(), FmError> {
        let vfs = self.backends.require(&uri)?;
        let token = self.editors.token();
        let mut doc = EditorDoc::new(uri.clone(), self.prefs.editor.undo_steps());
        doc.token = token;
        doc.wrap = self.prefs.editor.wrap;
        doc.line_numbers = self.prefs.editor.line_numbers;
        doc.restore = caret;
        doc.scroll_line = top;
        doc.reveal = true;
        doc.want_focus = true;
        doc.followed = self.preview_subject(pane).map(|e| e.uri);
        if is_archive_member(&uri) {
            doc.read_only = Some("archive".into());
        }
        if let Some(old) = self.editors.docs.get(&pane) {
            // Reloading the same file keeps the view settings the user chose.
            if old.uri == uri {
                doc.wrap = old.wrap;
                doc.line_numbers = old.line_numbers;
                doc.followed = old.followed.clone();
            }
            if let Some(job) = old.load_job {
                self.jobs.cancel(job);
            }
        }
        let inbox = self.editors.inbox.clone();
        let target = uri.clone();
        let job = self
            .jobs
            .submit(crate::backends::lane_for(uri.scheme()), move |ctx| {
                editor_io::load(&*vfs, &target, pane, token, &inbox, &|| ctx.is_cancelled());
                Ok(())
            });
        doc.load_job = Some(job);
        let view = EditViewState {
            uri,
            line: caret.map_or(0, |p| p.line),
            col: caret.map_or(0, |p| p.col),
            top,
            wrap: doc.wrap,
            folds: Vec::new(),
        };
        self.editors.docs.insert(pane, doc);
        if let Some(state) = self.layout.state_mut(pane) {
            state.edit = Some(view);
        }
        Ok(())
    }

    /// Reads the file again, keeping caret line and scroll (outside change, discard).
    pub(crate) fn editor_reload(&mut self, pane: PaneId) -> Result<(), FmError> {
        let Some(doc) = self.editors.docs.get(&pane) else {
            return Ok(());
        };
        let caret = doc.buffer.char_to_pos(doc.caret).ok();
        let (uri, top) = (doc.uri.clone(), doc.scroll_line);
        let folds = doc.folds.starts();
        self.editor_load(pane, uri, caret, top)?;
        if let Some(doc) = self.editors.docs.get_mut(&pane) {
            doc.fold_restore = Some(folds);
        }
        Ok(())
    }

    /// Back to the rendered preview; the document is dropped (its edits are saved or
    /// discarded by then).
    pub(crate) fn editor_close(&mut self, pane: PaneId) {
        if let Some(doc) = self.editors.docs.remove(&pane)
            && let Some(job) = doc.load_job
        {
            self.jobs.cancel(job);
        }
        if let Some(state) = self.layout.state_mut(pane) {
            state.edit = None;
        }
    }

    /// Starts writing the buffer. `to` is a Save as target; `check` compares with the
    /// file on disk first (off only when the user chose to overwrite).
    pub(crate) fn editor_save(
        &mut self,
        pane: PaneId,
        to: Option<VfsUri>,
        check: bool,
    ) -> Result<(), FmError> {
        self.editor_save_to(pane, to, check, false)
    }

    /// [`Self::editor_save`]; `refuse_existing` makes a Save as fail on an existing file.
    pub(crate) fn editor_save_to(
        &mut self,
        pane: PaneId,
        to: Option<VfsUri>,
        check: bool,
        refuse_existing: bool,
    ) -> Result<(), FmError> {
        let token = self.editors.token();
        let doc = self
            .editors
            .docs
            .get_mut(&pane)
            .ok_or(FmError::Unsupported("this pane is not editing a file"))?;
        if doc.phase == Phase::Saving {
            return Ok(());
        }
        if !doc.savable() {
            return Err(match &doc.read_only {
                Some(r) if r == "archive" => {
                    FmError::Unsupported("files inside an archive cannot be saved")
                }
                _ => FmError::Unsupported("this file cannot be saved"),
            });
        }
        let save_as = to.is_some();
        let dest = to.unwrap_or_else(|| doc.file().clone());
        if is_archive_member(&dest) {
            return Err(FmError::Unsupported(
                "files inside an archive cannot be saved",
            ));
        }
        let vfs = self.backends.require(&dest)?;
        let job = SaveJob {
            pane,
            token,
            to: dest.clone(),
            buffer: doc.buffer.clone(),
            encoding: doc.encoding,
            expect: if check && !save_as {
                doc.baseline.clone()
            } else {
                None
            },
            permissions: doc.baseline.as_ref().and_then(|b| b.permissions),
            refuse_existing,
        };
        doc.phase = Phase::Saving;
        doc.save_gen = token;
        doc.conflict = None;
        doc.save_error = None;
        let inbox = self.editors.inbox.clone();
        self.jobs
            .submit(crate::backends::lane_for(dest.scheme()), move |ctx| {
                editor_io::save(&*vfs, job, &inbox, &|| ctx.is_cancelled());
                Ok(())
            });
        Ok(())
    }

    /// Autosave, or the leave warning when autosave is off. `pending` runs once the
    /// buffer is clean; `None` only saves (focus moved away).
    pub(crate) fn editor_leave(&mut self, pane: PaneId, pending: Option<Pending>) {
        let autosave = self.prefs.editor.autosave;
        enum Next {
            Run,
            Save,
            Wait,
        }
        let next = {
            let Some(doc) = self.editors.docs.get_mut(&pane) else {
                return;
            };
            if pending.is_some() {
                doc.pending = pending;
            }
            if !doc.dirty() || !doc.savable() {
                Next::Run
            } else if doc.conflict.is_some() || doc.leave_prompt || doc.phase == Phase::Saving {
                // Already saving or waiting for the user.
                Next::Wait
            } else if !autosave {
                doc.leave_prompt = true;
                Next::Wait
            } else if doc.autosave_failed == Some(doc.buffer.version()) {
                // The last autosave of this text failed and its error is on screen: ask
                // instead of failing again on every focus change.
                if doc.pending.is_some() {
                    doc.leave_prompt = true;
                }
                Next::Wait
            } else {
                Next::Save
            }
        };
        match next {
            Next::Run => self.editor_run_pending(pane),
            Next::Save => {
                if let Err(e) = self.editor_save(pane, None, true) {
                    self.notice = Some(e.to_string());
                }
            }
            Next::Wait => {}
        }
    }

    /// Runs what waited for a clean buffer.
    fn editor_run_pending(&mut self, pane: PaneId) {
        let (pending, uri) = {
            let Some(doc) = self.editors.docs.get_mut(&pane) else {
                return;
            };
            if doc.dirty() && doc.savable() {
                return;
            }
            let Some(pending) = doc.pending.take() else {
                return;
            };
            (pending, doc.uri.clone())
        };
        match pending {
            Pending::Leave => self.editor_close(pane),
            Pending::Open {
                uri,
                line,
                activate,
            } => {
                let caret = line.map(|l| Pos {
                    line: l.saturating_sub(1),
                    col: 0,
                });
                match self.editor_load(pane, uri, caret, 0) {
                    Ok(()) => {
                        if let Some(doc) = self.editor_mut(pane) {
                            doc.activated = activate;
                        }
                    }
                    Err(e) => self.notice = Some(e.to_string()),
                }
            }
            Pending::External => {
                let params = serde_json::json!({ "uri": uri.to_string() });
                if let Err(e) = self.dispatch("file.open", &params) {
                    self.notice = Some(e.to_string());
                }
            }
        }
    }

    /// The answer to the leave warning, the conflict prompt or the changed-on-disk bar.
    pub(crate) fn editor_resolve(&mut self, pane: PaneId, choice: &str) -> Result<(), FmError> {
        if !matches!(
            choice,
            "save" | "discard" | "cancel" | "reload" | "overwrite" | "keep"
        ) {
            return Err(FmError::InvalidParams(format!(
                "choice must be save, discard, cancel, reload, overwrite or keep, not {choice:?}"
            )));
        }
        let doc = self
            .editors
            .docs
            .get_mut(&pane)
            .ok_or(FmError::Unsupported("this pane is not editing a file"))?;
        match choice {
            "save" => {
                doc.leave_prompt = false;
                self.editor_save(pane, None, true)
            }
            "discard" => {
                doc.leave_prompt = false;
                doc.declined = None;
                if doc.pending.is_some() {
                    // The edits go; what waited runs on the file as it is on disk.
                    doc.clean_version = doc.buffer.version();
                    self.editor_run_pending(pane);
                    Ok(())
                } else {
                    self.editor_reload(pane)
                }
            }
            "cancel" => {
                // Stay: `declined` keeps the file the user tried to move to, so the
                // warning does not come straight back.
                doc.leave_prompt = false;
                doc.pending = None;
                doc.want_focus = true;
                self.editors.last_focus = Some(pane);
                self.cancel_pending_close();
                self.layout.focus_pane(pane)
            }
            "reload" => {
                doc.conflict = None;
                doc.changed_on_disk = false;
                doc.pending = None;
                self.editor_reload(pane)
            }
            "overwrite" => {
                doc.conflict = None;
                doc.changed_on_disk = false;
                self.editor_save(pane, None, false)
            }
            _ => {
                // Keep mine: the next save replaces the file without asking again.
                doc.changed_on_disk = false;
                if let Some(disk) = doc.disk.take() {
                    doc.baseline = Some(disk);
                }
                Ok(())
            }
        }
    }

    /// A close or quit waiting for editor saves gives up (nothing is lost).
    fn cancel_pending_close(&mut self) {
        if self
            .pending_close
            .as_ref()
            .is_some_and(|p| !p.editors.is_empty())
        {
            self.pending_close = None;
        }
    }

    /// Unsaved edits of Preview pane `pane` keep it on its file; `desired` is what the
    /// source Files pane now points at. Called from `advance_slot`.
    pub(crate) fn editor_holds(&mut self, pane: PaneId, desired: &[Entry]) -> bool {
        let Some(doc) = self.editors.docs.get(&pane) else {
            return false;
        };
        if !doc.holds() {
            return false;
        }
        let Some(subject) = desired.first().map(|e| e.uri.clone()) else {
            return true;
        };
        // Only a move of the source counts: the first subject seen (what the source
        // showed when the editor opened, or a listing that arrived late) arms following,
        // and an unchanged subject is no move.
        let moved = doc.followed.as_ref().is_some_and(|f| *f != subject);
        let leave = moved
            && subject != doc.uri
            && doc.declined.as_ref() != Some(&subject)
            && doc.phase != Phase::Saving;
        if let Some(doc) = self.editors.docs.get_mut(&pane) {
            doc.followed = Some(subject.clone());
            if leave {
                doc.declined = Some(subject);
            }
        }
        if leave {
            self.editor_leave(pane, None);
        }
        true
    }

    /// The window lost focus: autosave every dirty editor (no warning when autosave is
    /// off: switching applications is not leaving the editor).
    pub fn editor_window_blur(&mut self) {
        if !self.prefs.editor.autosave {
            return;
        }
        let dirty: Vec<PaneId> = self
            .editors
            .docs
            .iter()
            .filter(|(_, d)| {
                d.dirty()
                    && d.savable()
                    && d.phase == Phase::Ready
                    && d.conflict.is_none()
                    && d.autosave_failed != Some(d.buffer.version())
            })
            .map(|(p, _)| *p)
            .collect();
        for pane in dirty {
            if let Err(e) = self.editor_save(pane, None, true) {
                self.notice = Some(e.to_string());
            }
        }
    }

    /// The undo depth preference changed: open editors follow at once.
    pub(crate) fn apply_editor_undo_steps(&mut self) {
        let steps = self.prefs.editor.undo_steps();
        for doc in self.editors.docs.values_mut() {
            doc.undo.set_max_steps(steps);
        }
    }

    /// Colouring was switched on/off or its size limit changed.
    pub(crate) fn editor_syntax_prefs_changed(&mut self) {
        let (on, max) = (
            self.prefs.editor.syntax,
            self.prefs.editor.syntax_max_bytes(),
        );
        for doc in self.editors.docs.values_mut() {
            if doc.phase != Phase::Loading {
                set_syntax(doc, on, max);
            }
        }
    }

    /// Something in `dir` changed on disk: editors of files there look again.
    pub(crate) fn editor_dir_changed(&mut self, dir: &VfsUri) {
        for doc in self.editors.docs.values_mut() {
            if doc.file().parent().as_ref() == Some(dir) || doc.uri.parent().as_ref() == Some(dir) {
                doc.check_due = true;
            }
        }
    }

    /// Once per poll: apply job results, react to focus changes, keep Edit mode in step
    /// with pane state, follow the source while clean, start parses and disk checks.
    pub(crate) fn pump_editors(&mut self) -> bool {
        let msgs = self
            .editors
            .inbox
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        let mut changed = !msgs.is_empty();
        for msg in msgs {
            self.apply_editor_msg(msg);
        }
        changed |= self.editor_focus_tick();
        changed |= self.editor_sync_panes();
        changed |= self.editor_follow();
        self.editor_start_parses();
        self.editor_start_checks();
        self.editor_write_view_state();
        changed
    }

    /// Focus moved to `pane` on purpose (opening a file there): not a focus loss.
    pub(crate) fn editor_focus_set(&mut self, pane: PaneId) {
        let prev = self.editors.last_focus.replace(pane);
        if let Some(prev) = prev.filter(|p| *p != pane && self.editors.docs.contains_key(p)) {
            self.editor_leave(prev, None);
        }
    }

    fn editor_focus_tick(&mut self) -> bool {
        let focused = self.layout.focused();
        let last = self.editors.last_focus.replace(focused);
        match last {
            Some(prev) if prev != focused && self.editors.docs.contains_key(&prev) => {
                self.editor_leave(prev, None);
                true
            }
            _ => false,
        }
    }

    /// Pane state says which panes edit what (a restored workspace sets it); documents of
    /// closed panes go.
    fn editor_sync_panes(&mut self) -> bool {
        let mut changed = false;
        let parked = crate::workspace::background_panes(&self.workspaces);
        let gone: Vec<PaneId> = self
            .editors
            .docs
            .keys()
            .copied()
            .filter(|p| self.layout.state(*p).is_none() && !parked.contains(p))
            .collect();
        for pane in gone {
            if let Some(doc) = self.editors.docs.remove(&pane)
                && let Some(job) = doc.load_job
            {
                self.jobs.cancel(job);
            }
            changed = true;
        }
        let restore: Vec<(PaneId, EditViewState)> = self
            .layout
            .states
            .iter()
            .filter(|(p, st)| st.kind == PaneKind::Preview && !self.editors.docs.contains_key(p))
            .filter_map(|(p, st)| st.edit.clone().map(|e| (*p, e)))
            .collect();
        for (pane, view) in restore {
            let caret = Pos {
                line: view.line,
                col: view.col,
            };
            if let Err(e) = self.editor_load(pane, view.uri, Some(caret), view.top) {
                self.notice = Some(e.to_string());
                if let Some(state) = self.layout.state_mut(pane) {
                    state.edit = None;
                }
            } else if let Some(doc) = self.editors.docs.get_mut(&pane) {
                doc.want_focus = false;
                doc.wrap = view.wrap;
                doc.fold_restore = Some(view.folds);
            }
            changed = true;
        }
        // A pane that stopped being a Preview leaves Edit mode once nothing is lost.
        let demoted: Vec<PaneId> = self
            .editors
            .docs
            .iter()
            .filter(|(p, d)| {
                self.layout
                    .state(**p)
                    .is_some_and(|st| st.kind != PaneKind::Preview || st.edit.is_none())
                    && !d.holds()
            })
            .map(|(p, _)| *p)
            .collect();
        for pane in demoted {
            self.editor_close(pane);
            changed = true;
        }
        changed
    }

    /// A clean editor follows its source Files pane like a preview does: another text
    /// file opens in Edit mode, anything else ends Edit mode.
    fn editor_follow(&mut self) -> bool {
        let mut moves = Vec::new();
        let mut arm = Vec::new();
        for (pane, doc) in &self.editors.docs {
            if doc.holds() || doc.phase != Phase::Ready || doc.pending.is_some() {
                continue;
            }
            let live = self
                .layout
                .state(*pane)
                .is_some_and(|st| matches!(st.preview_source, fm_layout::PreviewSource::Editor(_)));
            if live {
                continue;
            }
            let subject = self.preview_subject(*pane);
            let uri = subject.as_ref().map(|e| e.uri.clone());
            if uri == doc.followed || uri.is_none() {
                continue;
            }
            // The first file a new or restored pane shows only arms following: it is
            // what the source pointed at when the editor opened, not a move.
            moves.push((*pane, subject.filter(|_| doc.followed.is_some())));
            if doc.followed.is_none() {
                arm.push((*pane, uri));
            }
        }
        for (pane, uri) in arm {
            if let Some(doc) = self.editors.docs.get_mut(&pane) {
                doc.followed = uri;
            }
        }
        let changed = !moves.is_empty();
        for (pane, subject) in moves {
            let Some(entry) = subject else {
                continue;
            };
            if let Some(doc) = self.editors.docs.get_mut(&pane) {
                doc.followed = Some(entry.uri.clone());
                doc.declined = None;
                if doc.uri == entry.uri {
                    continue;
                }
            }
            if check_editable(&entry).is_ok() {
                let focus = self.editors.docs.get(&pane).is_some_and(|d| d.want_focus);
                if let Err(e) = self.editor_load(pane, entry.uri.clone(), None, 0) {
                    self.notice = Some(e.to_string());
                } else if let Some(doc) = self.editors.docs.get_mut(&pane) {
                    doc.want_focus = focus;
                    doc.followed = Some(entry.uri);
                }
            } else {
                self.editor_close(pane);
            }
        }
        changed
    }

    fn editor_start_parses(&mut self) {
        let parsing = self.editors.parsing.clone();
        let inbox = self.editors.inbox.clone();
        for (pane, doc) in self.editors.docs.iter_mut() {
            if parsing.load(Ordering::Relaxed) >= MAX_PARSERS {
                return;
            }
            let Some(syntax) = doc.syntax.as_ref() else {
                continue;
            };
            if doc.phase == Phase::Loading
                || doc.parse_inflight.is_some()
                || syntax.is_current(&doc.buffer)
            {
                continue;
            }
            let job = syntax.job(&doc.buffer, None);
            let ctl = doc.parse_ctl.clone();
            let (pane, token) = (*pane, doc.token);
            doc.parse_inflight = Some(job.version());
            parsing.fetch_add(1, Ordering::Relaxed);
            let counter = parsing.clone();
            let out = inbox.clone();
            let spawned = std::thread::Builder::new()
                .name("fm-parse".into())
                .spawn(move || {
                    let result = job.run(&ctl);
                    let folds = result
                        .as_ref()
                        .map(|(tree, _)| fm_editor::fold_tree_ranges(tree))
                        .unwrap_or_default();
                    counter.fetch_sub(1, Ordering::Relaxed);
                    editor_io::push(
                        &out,
                        EdMsg::Parsed {
                            pane,
                            token,
                            result,
                            folds,
                        },
                    );
                });
            if spawned.is_err() {
                parsing.fetch_sub(1, Ordering::Relaxed);
                doc.parse_inflight = None;
                doc.syntax = None;
                doc.plain_reason = Some("Syntax colouring is off: no worker available".into());
            }
        }
    }

    fn editor_start_checks(&mut self) {
        let due: Vec<(PaneId, u64, VfsUri)> = self
            .editors
            .docs
            .iter()
            .filter(|(_, d)| d.check_due && !d.check_inflight && d.phase == Phase::Ready)
            .map(|(p, d)| (*p, d.token, d.file().clone()))
            .collect();
        for (pane, token, real) in due {
            let Ok(vfs) = self.backends.require(&real) else {
                continue;
            };
            if let Some(doc) = self.editors.docs.get_mut(&pane) {
                doc.check_due = false;
                doc.check_inflight = true;
            }
            let inbox = self.editors.inbox.clone();
            self.jobs
                .submit(crate::backends::lane_for(real.scheme()), move |_ctx| {
                    editor_io::check(&*vfs, &real, pane, token, &inbox);
                    Ok(())
                });
        }
    }

    /// Caret and scroll go into pane state, so a saved workspace reopens there.
    fn editor_write_view_state(&mut self) {
        for (pane, doc) in &self.editors.docs {
            let Some(state) = self.layout.states.get_mut(pane) else {
                continue;
            };
            let Some(view) = state.edit.as_mut() else {
                continue;
            };
            if doc.phase == Phase::Loading {
                continue;
            }
            let pos = doc.buffer.char_to_pos(doc.caret).unwrap_or_default();
            let folds = doc
                .fold_restore
                .clone()
                .unwrap_or_else(|| doc.folds.starts());
            if view.uri != doc.uri
                || view.line != pos.line
                || view.col != pos.col
                || view.top != doc.scroll_line
                || view.wrap != doc.wrap
                || view.folds != folds
            {
                *view = EditViewState {
                    uri: doc.uri.clone(),
                    line: pos.line,
                    col: pos.col,
                    top: doc.scroll_line,
                    wrap: doc.wrap,
                    folds,
                };
            }
        }
    }

    fn apply_editor_msg(&mut self, msg: EdMsg) {
        let (on, max) = (
            self.prefs.editor.syntax,
            self.prefs.editor.syntax_max_bytes(),
        );
        match msg {
            EdMsg::Opened {
                pane,
                token,
                real,
                encoding,
                baseline,
            } => {
                if let Some(doc) = self.live_doc(pane, token) {
                    doc.real = Some(real);
                    doc.encoding = encoding;
                    doc.total = baseline.size;
                    doc.baseline = Some(baseline);
                }
            }
            EdMsg::Blocked { pane, token, why } => {
                let mut hand_off = None;
                if let Some(doc) = self.live_doc(pane, token) {
                    // A double-clicked file of unknown type that is not text after all
                    // opens as before, in its default application.
                    if hands_off_to_app(doc.activated, &why) {
                        hand_off = Some(doc.uri.clone());
                    }
                    doc.blocked = Some(why);
                    doc.phase = Phase::Ready;
                    doc.load_job = None;
                }
                if let Some(uri) = hand_off {
                    self.editor_close(pane);
                    let params = serde_json::json!({ "uri": uri.to_string() });
                    if let Err(e) = self.dispatch("file.open", &params) {
                        self.notice = Some(e.to_string());
                    }
                }
            }
            EdMsg::Restart {
                pane,
                token,
                encoding,
            } => {
                if let Some(doc) = self.live_doc(pane, token) {
                    doc.buffer = Buffer::new();
                    doc.encoding = encoding;
                    doc.loaded = 0;
                }
            }
            EdMsg::Text {
                pane,
                token,
                text,
                loaded,
            } => {
                if let Some(doc) = self.live_doc(pane, token) {
                    doc.buffer.append(&text);
                    doc.loaded = loaded;
                }
            }
            EdMsg::Loaded {
                pane,
                token,
                result,
            } => {
                if let Some(doc) = self.live_doc(pane, token) {
                    doc.load_job = None;
                    doc.phase = Phase::Ready;
                    match result {
                        Ok(()) => finish_load(doc, on, max),
                        Err(e) => doc.load_error = Some(e),
                    }
                }
            }
            EdMsg::Saved {
                pane,
                token,
                version,
                to,
                result,
            } => self.editor_saved(pane, token, version, to, result),
            EdMsg::Found {
                pane,
                token,
                rev,
                version,
                result,
            } => self.editor_found(pane, token, rev, version, result),
            EdMsg::Parsed {
                pane,
                token,
                result,
                folds,
            } => {
                let Some(doc) = self.live_doc(pane, token) else {
                    return;
                };
                doc.parse_inflight = None;
                match result {
                    Ok((tree, version)) => {
                        if let Some(syn) = doc.syntax.as_mut() {
                            syn.install(tree, version);
                        }
                        // Ranges of an older text stay shifted until the next parse.
                        if version == doc.buffer.version() {
                            doc.fold_ranges = fm_editor::FoldRanges::from_tree(folds, version);
                            restore_folds(doc);
                        }
                    }
                    Err(SyntaxError::Cancelled) => {}
                    Err(SyntaxError::TimedOut) => {
                        doc.syntax = None;
                        doc.fold_ranges = fm_editor::FoldRanges::default();
                        restore_folds(doc);
                        doc.plain_reason =
                            Some("Syntax colouring is off: the file took too long to parse".into());
                    }
                    Err(SyntaxError::Unsupported) => doc.syntax = None,
                }
            }
            EdMsg::Checked {
                pane,
                token,
                result,
            } => {
                let reload = {
                    let Some(doc) = self.live_doc(pane, token) else {
                        return;
                    };
                    doc.check_inflight = false;
                    let Ok(now) = result else {
                        return;
                    };
                    let changed = match (&now, &doc.baseline) {
                        (Some(now), Some(base)) => now.differs(base),
                        (None, Some(_)) => true,
                        _ => false,
                    };
                    if !changed || doc.phase != Phase::Ready {
                        return;
                    }
                    if doc.dirty() || now.is_none() {
                        doc.changed_on_disk = true;
                        doc.disk = now;
                        false
                    } else {
                        true
                    }
                };
                if reload && let Err(e) = self.editor_reload(pane) {
                    self.notice = Some(e.to_string());
                }
            }
        }
    }

    /// The document `pane` edits, if `token` is its current load (late results of an
    /// older file are dropped).
    fn live_doc(&mut self, pane: PaneId, token: u64) -> Option<&mut EditorDoc> {
        self.editors
            .docs
            .get_mut(&pane)
            .filter(|d| d.token == token)
    }

    fn editor_saved(
        &mut self,
        pane: PaneId,
        token: u64,
        version: u64,
        to: VfsUri,
        result: Result<crate::editor_doc::Baseline, SaveFail>,
    ) {
        let Some(doc) = self
            .editors
            .docs
            .get_mut(&pane)
            .filter(|d| d.save_gen == token)
        else {
            return;
        };
        doc.phase = Phase::Ready;
        match result {
            Ok(baseline) => {
                if &to != doc.file() {
                    // Save as: the editor now edits the new file.
                    doc.name = to.file_name().unwrap_or("file").to_string();
                    doc.uri = to.clone();
                    doc.real = Some(to.clone());
                    doc.read_only = None;
                    if let Some(state) = self.layout.states.get_mut(&pane)
                        && let Some(view) = state.edit.as_mut()
                    {
                        view.uri = to.clone();
                    }
                }
                doc.baseline = Some(baseline);
                doc.clean_version = version;
                doc.conflict = None;
                doc.changed_on_disk = false;
                doc.disk = None;
                doc.save_error = None;
                doc.autosave_failed = None;
                doc.declined = None;
                let uri = doc.uri.clone();
                self.invalidate_preview_of(&uri);
                if let Some(parent) = to.parent()
                    && self.dirs.entries(&parent).is_some()
                {
                    let _ = self.refresh_read(parent);
                }
                self.editor_run_pending(pane);
                self.editor_close_progress();
            }
            Err(SaveFail::Conflict(c)) => {
                doc.conflict = Some(c);
                self.cancel_pending_close();
            }
            Err(SaveFail::Error(e)) => {
                doc.autosave_failed = Some(version);
                doc.pending = None;
                doc.save_error = Some(e.clone());
                self.notice = Some(format!("{} was not saved: {e}", doc.name));
                self.cancel_pending_close();
            }
            Err(SaveFail::Cancelled) => {
                doc.pending = None;
                self.cancel_pending_close();
            }
        }
    }
}

/// Settings that depend on the whole text, once it has loaded.
fn finish_load(doc: &mut EditorDoc, syntax_on: bool, syntax_max: usize) {
    let head: String = {
        let mut out = String::new();
        for chunk in doc.buffer.chunks() {
            out.push_str(chunk);
            if out.len() >= DETECT_CHARS {
                break;
            }
        }
        out
    };
    let (eol, mixed) = fm_editor::detect_eol(&head);
    doc.eol = eol;
    doc.mixed_eol = mixed;
    doc.indent = fm_editor::detect_indent(&doc.buffer);
    let first = head.lines().next().unwrap_or("");
    doc.lang = fm_editor::detect_lang(&doc.name.to_ascii_lowercase(), first);
    doc.undo.clear();
    doc.clean_version = doc.buffer.version();
    set_syntax(doc, syntax_on, syntax_max);
    if let Some(pos) = doc.restore.take() {
        let last = doc.buffer.len_lines().saturating_sub(1);
        let line = pos.line.min(last);
        let col = pos.col.min(doc.buffer.line_len(line).unwrap_or(0));
        doc.caret = doc.buffer.pos_to_char(Pos { line, col }).unwrap_or(0);
        doc.anchor = doc.caret;
        doc.scroll_line = doc.scroll_line.min(last);
    }
    doc.clamp_cursor();
    if doc.syntax.is_none() {
        restore_folds(doc);
    }
}

/// Puts back remembered folds once it is known where blocks are.
fn restore_folds(doc: &mut EditorDoc) {
    if let Some(starts) = doc.fold_restore.take() {
        doc.folds.restore(&starts, &doc.fold_ranges, &doc.buffer);
        doc.caret_out_of_folds();
    }
}

fn set_syntax(doc: &mut EditorDoc, on: bool, max: usize) {
    doc.parse_ctl.cancel.store(true, Ordering::Relaxed);
    doc.parse_ctl = fm_editor::ParseCtl::default();
    doc.parse_inflight = None;
    doc.syntax = None;
    doc.plain_reason = None;
    doc.fold_ranges = fm_editor::FoldRanges::default();
    let Some(lang) = doc.lang else {
        return;
    };
    if !on {
        doc.plain_reason = Some("Syntax colouring is off in Preferences".into());
    } else if doc.buffer.len_bytes() > max {
        doc.plain_reason = Some("Syntax colouring is off for large files".into());
    } else if !lang.available() {
        doc.plain_reason = Some(format!("No colouring for {} in this build", lang.label()));
    } else {
        doc.syntax = Syntax::new(lang).ok();
    }
}

/// The language of a file name, for labels.
pub fn lang_label(lang: Option<Lang>) -> &'static str {
    lang.map_or("Plain text", Lang::label)
}
