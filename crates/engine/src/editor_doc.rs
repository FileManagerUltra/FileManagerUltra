//! One document open in the Edit mode of a Preview pane: text, undo history, caret,
//! encoding and the bookkeeping that keeps it safe to save (capability `text-editor`).
//!
//! Pure data and pure edits: no I/O happens here. Loading and saving are jobs in
//! `editor_io`; the commands in `editor_cmds` call these methods.

use std::time::SystemTime;

use fm_editor::{
    Buffer, Coalesce, EditRecord, Encoding, Eol, IndentStyle, Lang, Syntax, UndoLog, convert_eol,
};
use fm_vfs::{Entry, FmError, VfsUri};

/// Largest file the editor opens; bigger ones stay in Preview.
pub const MAX_EDIT_BYTES: u64 = 1 << 30;
/// Largest text one `edit.input` (paste) may insert.
pub const MAX_INPUT_BYTES: usize = 64 << 20;

/// What was known about the file when it was loaded or last saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Baseline {
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub permissions: Option<u32>,
}

impl Baseline {
    pub fn of(entry: &Entry) -> Self {
        Self {
            size: entry.size,
            modified: entry.modified,
            permissions: entry.permissions,
        }
    }

    /// Whether the file on disk is no longer the one that was loaded.
    pub fn differs(&self, other: &Self) -> bool {
        self.size != other.size || self.modified != other.modified
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Loading,
    Ready,
    Saving,
}

/// Why a save did not happen and needs the user to decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// The file changed on disk since it was loaded.
    Changed,
    /// The file is gone.
    Deleted,
}

/// Cursor movements of `edit.move`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    WordLeft,
    WordRight,
    PageUp,
    PageDown,
}

impl Move {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "left" => Self::Left,
            "right" => Self::Right,
            "up" => Self::Up,
            "down" => Self::Down,
            "line_start" => Self::LineStart,
            "line_end" => Self::LineEnd,
            "doc_start" => Self::DocStart,
            "doc_end" => Self::DocEnd,
            "word_left" => Self::WordLeft,
            "word_right" => Self::WordRight,
            "page_up" => Self::PageUp,
            "page_down" => Self::PageDown,
            _ => return None,
        })
    }
}

/// Find bar state of one editor.
#[derive(Clone, Debug, Default)]
pub struct EditFind {
    pub open: bool,
    pub replace: bool,
    pub query: String,
    pub replacement: String,
    pub options: fm_editor::FindOptions,
    /// Matches of the last "find all", for the counter and highlights.
    pub matches: Vec<fm_editor::Match>,
    pub truncated: bool,
    pub error: Option<String>,
    /// Buffer version `matches` were computed for.
    pub version: u64,
    /// Bumped when the query or options change, so a late count is dropped.
    pub(crate) rev: u64,
    /// A count runs on a worker (large files).
    pub busy: bool,
}

pub struct EditorDoc {
    pub uri: VfsUri,
    pub name: String,
    pub buffer: Buffer,
    pub undo: UndoLog,
    pub syntax: Option<Syntax>,
    pub lang: Option<Lang>,
    pub encoding: Encoding,
    pub eol: Eol,
    pub mixed_eol: bool,
    pub indent: IndentStyle,
    pub phase: Phase,
    /// Why the buffer cannot be changed (binary file, archive member, …).
    pub read_only: Option<String>,
    /// Why the file is shown without colouring.
    pub plain_reason: Option<String>,
    /// Buffer version that equals the file on disk.
    pub clean_version: u64,
    pub caret: usize,
    pub anchor: usize,
    pub(crate) goal_col: Option<usize>,
    /// Topmost visible line, for restoring the view.
    pub scroll_line: usize,
    pub wrap: bool,
    pub line_numbers: bool,
    pub baseline: Option<Baseline>,
    pub conflict: Option<Conflict>,
    /// The file changed on disk while the buffer has unsaved edits.
    pub changed_on_disk: bool,
    /// Autosave is off and the user left the pane with unsaved edits.
    pub leave_prompt: bool,
    pub goto_open: bool,
    pub find: EditFind,
    /// Bytes read / total while loading.
    pub loaded: u64,
    pub total: Option<u64>,
    pub load_error: Option<String>,
    pub save_error: Option<String>,
    /// Bumped per load so late results of an older one are ignored.
    pub(crate) token: u64,
    pub(crate) parse_inflight: Option<u64>,
    pub(crate) check_inflight: bool,
    /// Caret line to scroll into view on the next frame.
    pub reveal: bool,
    /// Take keyboard focus on the next frame.
    pub want_focus: bool,
    /// The Preview subject this pane last followed; a different one moves a clean editor.
    pub(crate) followed: Option<VfsUri>,
    /// Caret to restore once a reload finishes.
    pub(crate) restore: Option<fm_editor::Pos>,
    pub(crate) save_gen: u64,
    /// The load stopped early (binary files show only their start).
    pub truncated: bool,
    /// The file actually read and written: `uri` with symlinks followed.
    pub real: Option<VfsUri>,
    /// Why there is no editable text (binary, too large, not a file).
    pub blocked: Option<crate::editor_io::Blocked>,
    /// What to do once the buffer is saved or its edits are discarded.
    pub pending: Option<Pending>,
    /// A file the user chose to stay away from (Cancel in the leave warning).
    pub(crate) declined: Option<VfsUri>,
    /// Buffer version whose autosave failed: no new attempt until the text changes.
    pub(crate) autosave_failed: Option<u64>,
    /// The file may have changed on disk: look at it on the next poll.
    pub(crate) check_due: bool,
    /// The file on disk as last seen, when it differs from `baseline`.
    pub(crate) disk: Option<Baseline>,
    pub(crate) parse_ctl: fm_editor::ParseCtl,
    pub(crate) load_job: Option<crate::jobs::JobId>,
    /// Folded blocks (view state; the text is untouched).
    pub folds: fm_editor::FoldIndex,
    /// Where blocks can fold: from the syntax tree, else indentation.
    pub fold_ranges: fm_editor::FoldRanges,
    /// Folds to put back once fold ranges are known (workspace restore, reload).
    pub(crate) fold_restore: Option<Vec<usize>>,
}

/// An action waiting for a save (or the user's decision) before it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pending {
    /// Open another file in this pane.
    Open { uri: VfsUri, line: Option<usize> },
    /// Back to the rendered preview.
    Leave,
    /// Hand the file to the default application.
    External,
}

impl Drop for EditorDoc {
    fn drop(&mut self) {
        // A parse still running for a closed editor stops at its next progress check.
        self.parse_ctl
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl EditorDoc {
    pub fn new(uri: VfsUri, undo_steps: usize) -> Self {
        let name = uri.file_name().unwrap_or("file").to_string();
        Self {
            uri,
            name,
            buffer: Buffer::new(),
            undo: UndoLog::new(undo_steps),
            syntax: None,
            lang: None,
            encoding: Encoding::Utf8,
            eol: Eol::Lf,
            mixed_eol: false,
            indent: IndentStyle::Spaces(4),
            phase: Phase::Loading,
            read_only: None,
            plain_reason: None,
            clean_version: 0,
            caret: 0,
            anchor: 0,
            goal_col: None,
            scroll_line: 0,
            wrap: false,
            line_numbers: true,
            baseline: None,
            conflict: None,
            changed_on_disk: false,
            leave_prompt: false,
            goto_open: false,
            find: EditFind::default(),
            loaded: 0,
            total: None,
            load_error: None,
            save_error: None,
            token: 0,
            parse_inflight: None,
            check_inflight: false,
            reveal: false,
            want_focus: false,
            followed: None,
            restore: None,
            save_gen: 0,
            truncated: false,
            real: None,
            blocked: None,
            pending: None,
            declined: None,
            autosave_failed: None,
            check_due: false,
            disk: None,
            parse_ctl: fm_editor::ParseCtl::default(),
            load_job: None,
            folds: fm_editor::FoldIndex::default(),
            fold_ranges: fm_editor::FoldRanges::default(),
            fold_restore: None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.buffer.version() != self.clean_version
    }

    /// The pane must stay on this file: unsaved work, a save in flight or a pending decision.
    pub fn holds(&self) -> bool {
        self.dirty() || self.phase == Phase::Saving || self.conflict.is_some() || self.leave_prompt
    }

    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection();
        (a != b).then(|| self.buffer.slice(a..b).ok()).flatten()
    }

    /// The file on disk this document reads and writes.
    pub fn file(&self) -> &VfsUri {
        self.real.as_ref().unwrap_or(&self.uri)
    }

    /// The buffer can be saved: loaded, editable text.
    pub fn savable(&self) -> bool {
        self.phase != Phase::Loading
            && self.blocked.is_none()
            && self.read_only.is_none()
            && self.load_error.is_none()
    }

    pub fn ensure_editable(&self) -> Result<(), FmError> {
        if self.blocked.is_some() {
            return Err(FmError::Unsupported("this file is not opened for editing"));
        }
        if let Some(why) = &self.read_only {
            return Err(FmError::Unsupported(match why.as_str() {
                "binary" => "this looks like a binary file; it is opened read-only",
                "archive" => "files inside an archive are read-only",
                _ => "this file is read-only",
            }));
        }
        if self.phase == Phase::Loading {
            return Err(FmError::Unsupported("the file is still loading"));
        }
        Ok(())
    }

    fn clamp(&self, idx: usize) -> usize {
        idx.min(self.buffer.len_chars())
    }

    /// Caret and selection stay inside the text after any change.
    pub(crate) fn clamp_cursor(&mut self) {
        self.caret = self.clamp(self.caret);
        self.anchor = self.clamp(self.anchor);
    }

    fn note(&mut self, rec: &EditRecord) {
        if let Some(syn) = self.syntax.as_mut() {
            syn.note_edit(&self.buffer, rec);
        }
        self.folds.note_edit(&self.buffer, rec);
        self.fold_ranges.note_edit(&self.buffer, rec);
    }

    /// The caret's line is never hidden: a fold around it opens.
    fn reveal_caret(&mut self) {
        if !self.folds.is_empty()
            && let Ok(pos) = self.buffer.char_to_pos(self.caret)
        {
            self.folds.reveal(pos.line);
        }
    }

    fn after_edit(&mut self, caret: usize) {
        self.caret = self.clamp(caret);
        self.anchor = self.caret;
        self.goal_col = None;
        self.reveal = true;
        self.save_error = None;
    }

    /// Moves an index across an edit that already happened.
    fn shift(idx: usize, rec: &EditRecord) -> usize {
        let removed = rec.removed.chars().count();
        let inserted = rec.inserted.chars().count();
        if idx >= rec.start_char + removed {
            idx + inserted - removed
        } else if idx > rec.start_char {
            rec.start_char
        } else {
            idx
        }
    }

    fn follow(&mut self, recs: &[EditRecord]) {
        for rec in recs {
            self.caret = Self::shift(self.caret, rec);
            self.anchor = Self::shift(self.anchor, rec);
        }
        self.clamp_cursor();
        self.goal_col = None;
        self.reveal = true;
        self.save_error = None;
    }

    fn record(&mut self, rec: EditRecord, kind: Coalesce) {
        self.note(&rec);
        self.undo.record(rec, kind);
    }

    fn record_group(&mut self, recs: Vec<EditRecord>) {
        for rec in &recs {
            self.note(rec);
        }
        self.undo.record_group(recs);
    }

    fn bad(e: impl std::fmt::Display) -> FmError {
        FmError::InvalidParams(e.to_string())
    }

    /// Types or pastes `text` over the selection.
    pub fn input(&mut self, text: &str) -> Result<(), FmError> {
        self.ensure_editable()?;
        if text.len() > MAX_INPUT_BYTES {
            return Err(FmError::InvalidParams(
                "the text is too large to insert".into(),
            ));
        }
        if text.is_empty() {
            return Ok(());
        }
        let text = if text.contains(['\r', '\n']) {
            convert_eol(text, self.eol)
        } else {
            text.to_string()
        };
        let (a, b) = self.selection();
        let at = if a != b {
            let rec = self.buffer.replace(a..b, &text).map_err(Self::bad)?;
            let caret = rec.start_char + text.chars().count();
            self.record(rec, Coalesce::No);
            caret
        } else {
            let rec = self.buffer.insert(a, &text).map_err(Self::bad)?;
            let caret = rec.start_char + text.chars().count();
            let single = text.chars().count() == 1 && !text.contains('\n');
            self.record(
                rec,
                if single {
                    Coalesce::Typing
                } else {
                    Coalesce::No
                },
            );
            caret
        };
        self.after_edit(at);
        Ok(())
    }

    /// Enter: a line break that keeps the indent.
    pub fn newline(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (a, b) = self.selection();
        let colon = self.lang.is_some_and(Lang::colon_opens_block);
        let text = fm_editor::newline_text(&self.buffer, a, self.indent, self.eol, colon)
            .map_err(Self::bad)?;
        let rec = self.buffer.replace(a..b, &text).map_err(Self::bad)?;
        let caret = rec.start_char + text.chars().count();
        self.record(rec, Coalesce::No);
        self.after_edit(caret);
        Ok(())
    }

    /// Backspace / Delete, by character or word; deletes the selection when there is one.
    pub fn delete(&mut self, forward: bool, word: bool) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (mut a, mut b) = self.selection();
        let mut kind = Coalesce::No;
        if a == b {
            kind = Coalesce::Deleting;
            if forward {
                b = if word {
                    self.word_right(a)
                } else {
                    (a + 1).min(self.buffer.len_chars())
                };
                // Delete across a CRLF as one.
                if self.buffer.char_at(a) == Some('\r') && self.buffer.char_at(a + 1) == Some('\n')
                {
                    b = b.max(a + 2);
                }
            } else {
                a = if word {
                    self.word_left(b)
                } else {
                    b.saturating_sub(1)
                };
                if a > 0
                    && self.buffer.char_at(a) == Some('\n')
                    && self.buffer.char_at(a - 1) == Some('\r')
                {
                    a -= 1;
                }
            }
            if a == b {
                return Ok(());
            }
            if word {
                kind = Coalesce::No;
            }
        }
        let rec = self.buffer.remove(a..b).map_err(Self::bad)?;
        self.record(rec, kind);
        self.after_edit(a);
        Ok(())
    }

    /// First and last line a line command acts on.
    pub fn line_span(&self) -> (usize, usize) {
        let (a, b) = self.selection();
        let first = self.buffer.char_to_pos(a).map_or(0, |p| p.line);
        let end = self.buffer.char_to_pos(b).unwrap_or_default();
        let last = if b > a && end.col == 0 && end.line > first {
            end.line - 1
        } else {
            end.line
        };
        (first, last.max(first))
    }

    pub fn indent_selection(&mut self, outdent: bool) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (first, last) = self.line_span();
        // Tab with a caret and no selection types the indent.
        if !outdent && !self.has_selection() {
            let unit = self.indent.unit();
            return self.input(&unit);
        }
        let recs = fm_editor::indent_lines(&mut self.buffer, first, last, self.indent, outdent)
            .map_err(Self::bad)?;
        if recs.is_empty() {
            return Ok(());
        }
        self.follow_group(recs);
        Ok(())
    }

    fn follow_group(&mut self, recs: Vec<EditRecord>) {
        self.follow(&recs);
        self.record_group(recs);
    }

    pub fn duplicate_lines(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (first, last) = self.line_span();
        let rec = fm_editor::duplicate_lines(&mut self.buffer, first, last).map_err(Self::bad)?;
        self.follow(std::slice::from_ref(&rec));
        self.record(rec, Coalesce::No);
        Ok(())
    }

    pub fn delete_lines(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (first, last) = self.line_span();
        let rec = fm_editor::delete_lines(&mut self.buffer, first, last).map_err(Self::bad)?;
        let at = rec.start_char;
        self.record(rec, Coalesce::No);
        self.after_edit(at);
        Ok(())
    }

    pub fn move_lines(&mut self, down: bool) -> Result<(), FmError> {
        self.ensure_editable()?;
        let (first, last) = self.line_span();
        let before = |idx: usize| self.buffer.char_to_pos(idx).unwrap_or_default();
        let (c, a) = (before(self.caret), before(self.anchor));
        let Some(rec) =
            fm_editor::move_lines(&mut self.buffer, first, last, down).map_err(Self::bad)?
        else {
            return Ok(());
        };
        self.record(rec, Coalesce::No);
        // The selection stays on the moved lines.
        let moved = |buf: &Buffer, pos: fm_editor::Pos| -> usize {
            let line = if down {
                pos.line + 1
            } else {
                pos.line.saturating_sub(1)
            };
            buf.pos_to_char(fm_editor::Pos { line, col: pos.col })
                .unwrap_or(0)
        };
        self.caret = moved(&self.buffer, c);
        self.anchor = moved(&self.buffer, a);
        self.reveal = true;
        Ok(())
    }

    pub fn toggle_comment(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        let prefix = self
            .lang
            .and_then(Lang::line_comment)
            .ok_or(FmError::Unsupported("this language has no line comment"))?;
        let (first, last) = self.line_span();
        let recs =
            fm_editor::toggle_comment(&mut self.buffer, first, last, prefix).map_err(Self::bad)?;
        if !recs.is_empty() {
            self.follow_group(recs);
        }
        Ok(())
    }

    pub fn undo(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        match self.undo.undo(&mut self.buffer) {
            Ok(Some(recs)) => {
                for r in &recs {
                    self.note(r);
                }
                if let Some(last) = recs.last() {
                    self.after_edit(last.start_char + last.inserted.chars().count());
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => Err(Self::bad(e)),
        }
    }

    pub fn redo(&mut self) -> Result<(), FmError> {
        self.ensure_editable()?;
        match self.undo.redo(&mut self.buffer) {
            Ok(Some(recs)) => {
                for r in &recs {
                    self.note(r);
                }
                if let Some(last) = recs.last() {
                    self.after_edit(last.start_char + last.inserted.chars().count());
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => Err(Self::bad(e)),
        }
    }

    /// Records a group of edits that a find/replace made.
    pub(crate) fn replaced(&mut self, recs: Vec<EditRecord>) {
        if recs.is_empty() {
            return;
        }
        self.follow_group(recs);
    }

    /// Replaces the whole text with `text` as one undoable step (line-ending change).
    pub fn replace_all_text(&mut self, text: &str) -> Result<(), FmError> {
        self.ensure_editable()?;
        let len = self.buffer.len_chars();
        let caret = self.caret.min(len);
        let rec = self.buffer.replace(0..len, text).map_err(Self::bad)?;
        self.record(rec, Coalesce::No);
        self.after_edit(caret);
        Ok(())
    }

    pub fn set_eol(&mut self, eol: Eol) -> Result<(), FmError> {
        self.ensure_editable()?;
        if self.eol == eol && !self.mixed_eol {
            return Ok(());
        }
        let text = convert_eol(&self.buffer.to_text(), eol);
        self.replace_all_text(&text)?;
        self.eol = eol;
        self.mixed_eol = false;
        Ok(())
    }

    pub fn goto_line(&mut self, line: usize) -> Result<(), FmError> {
        if line == 0 {
            return Err(FmError::InvalidParams("lines start at 1".into()));
        }
        let last = self.buffer.len_lines().saturating_sub(1);
        let at = self
            .buffer
            .line_start(line.saturating_sub(1).min(last))
            .map_err(Self::bad)?;
        self.caret = at;
        self.anchor = at;
        self.goal_col = None;
        self.undo.break_run();
        self.reveal = true;
        self.goto_open = false;
        self.reveal_caret();
        Ok(())
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.buffer.len_chars();
        self.undo.break_run();
    }

    /// Selects the word (or run of punctuation or spaces) around a char index.
    pub fn select_word(&mut self, idx: usize) {
        let idx = self.clamp(idx);
        let class = self.buffer.char_at(idx).map(char_class);
        let Some(class) = class.filter(|c| *c != Class::Newline) else {
            self.set_caret(idx, false);
            return;
        };
        let back = self
            .buffer
            .chars_before(idx)
            .into_iter()
            .flatten()
            .take_while(|c| char_class(*c) == class)
            .count();
        let ahead = self
            .buffer
            .chars_from(idx)
            .into_iter()
            .flatten()
            .take_while(|c| char_class(*c) == class)
            .count();
        self.set_caret(idx - back, false);
        self.set_caret(idx + ahead, true);
    }

    /// Places the caret at a char index (pointer click or drag).
    pub fn set_caret(&mut self, idx: usize, extend: bool) {
        self.caret = self.clamp(idx);
        if !extend {
            self.anchor = self.caret;
        }
        self.goal_col = None;
        self.undo.break_run();
        self.reveal_caret();
    }

    pub fn move_caret(&mut self, mv: Move, extend: bool, page: usize) -> Result<(), FmError> {
        let len = self.buffer.len_chars();
        let (sa, sb) = self.selection();
        let had = sa != sb;
        let mut keep_goal = false;
        let to = match mv {
            Move::Left if had && !extend => sa,
            Move::Right if had && !extend => sb,
            Move::Left => self.prev_char(self.caret),
            Move::Right => self.next_char(self.caret),
            Move::WordLeft => self.word_left(self.caret),
            Move::WordRight => self.word_right(self.caret),
            Move::DocStart => 0,
            Move::DocEnd => len,
            Move::LineStart => self.smart_home()?,
            Move::LineEnd => {
                let pos = self.buffer.char_to_pos(self.caret).map_err(Self::bad)?;
                self.buffer.line_start(pos.line).map_err(Self::bad)?
                    + self.buffer.line_len(pos.line).unwrap_or(0)
            }
            Move::Up | Move::Down | Move::PageUp | Move::PageDown => {
                keep_goal = true;
                let step = match mv {
                    Move::Up | Move::Down => 1,
                    _ => page.max(1),
                };
                let up = matches!(mv, Move::Up | Move::PageUp);
                let pos = self.buffer.char_to_pos(self.caret).map_err(Self::bad)?;
                let goal = *self.goal_col.get_or_insert(pos.col);
                let last = self.buffer.len_lines().saturating_sub(1);
                if (up && pos.line == 0) || (!up && pos.line >= last) {
                    // Past the first/last line the caret goes to the edge of the text.
                    if up { 0 } else { len }
                } else {
                    // Folded lines are stepped over, not opened.
                    let mut line = pos.line;
                    for _ in 0..step {
                        let next = if up {
                            self.folds.prev_visible(line)
                        } else {
                            self.folds.next_visible(line)
                        };
                        if next > last || next == line {
                            break;
                        }
                        line = next;
                    }
                    self.buffer
                        .pos_to_char(fm_editor::Pos { line, col: goal })
                        .map_err(Self::bad)?
                }
            }
        };
        let goal = self.goal_col;
        self.set_caret(to, extend);
        self.goal_col = if keep_goal { goal } else { None };
        self.reveal = true;
        Ok(())
    }

    fn prev_char(&self, idx: usize) -> usize {
        let mut i = idx.saturating_sub(1);
        if i > 0 && self.buffer.char_at(i) == Some('\n') && self.buffer.char_at(i - 1) == Some('\r')
        {
            i -= 1;
        }
        i
    }

    fn next_char(&self, idx: usize) -> usize {
        let len = self.buffer.len_chars();
        let mut i = (idx + 1).min(len);
        if self.buffer.char_at(idx) == Some('\r') && self.buffer.char_at(idx + 1) == Some('\n') {
            i = (idx + 2).min(len);
        }
        i
    }

    fn smart_home(&self) -> Result<usize, FmError> {
        let pos = self.buffer.char_to_pos(self.caret).map_err(Self::bad)?;
        let start = self.buffer.line_start(pos.line).map_err(Self::bad)?;
        let text = self.buffer.line_text(pos.line).unwrap_or_default();
        let indent = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        Ok(if pos.col == indent {
            start
        } else {
            start + indent
        })
    }

    fn word_left(&self, idx: usize) -> usize {
        let Some(before) = self.buffer.chars_before(idx) else {
            return idx;
        };
        let mut n = 0usize;
        for c in before {
            let class = char_class(c);
            if class == Class::Space {
                n += 1;
                continue;
            }
            if class == Class::Newline {
                if n == 0 {
                    n = 1;
                }
                break;
            }
            n += 1;
            // Continue while the class stays the same.
            let rest = self.buffer.chars_before(idx - n).into_iter().flatten();
            n += rest.take_while(|c2| char_class(*c2) == class).count();
            break;
        }
        idx.saturating_sub(n)
    }

    fn word_right(&self, idx: usize) -> usize {
        let Some(after) = self.buffer.chars_from(idx) else {
            return idx;
        };
        let mut n = 0usize;
        let mut class: Option<Class> = None;
        for c in after {
            let k = char_class(c);
            if k == Class::Newline {
                if n == 0 {
                    n = 1;
                }
                break;
            }
            match class {
                None if k == Class::Space => n += 1,
                None => {
                    class = Some(k);
                    n += 1;
                }
                Some(first) if k == first => n += 1,
                Some(_) => break,
            }
        }
        // Skip the whitespace after the word as editors do.
        let tail = self.buffer.chars_from(idx + n).into_iter().flatten();
        let spaces = tail.take_while(|c| char_class(*c) == Class::Space).count();
        (idx + n + spaces).min(self.buffer.len_chars())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Newline,
    Punct,
}

fn char_class(c: char) -> Class {
    if c == '\n' || c == '\r' {
        Class::Newline
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    fn doc(text: &str) -> EditorDoc {
        let mut d = EditorDoc::new(VfsUri::parse("/tmp/a.rs").expect("uri"), 100);
        d.buffer.append(text);
        d.clean_version = d.buffer.version();
        d.phase = Phase::Ready;
        d.lang = Some(Lang::Rust);
        d
    }

    #[test]
    fn typing_dirties_and_undo_restores_text() {
        let mut d = doc("hello");
        assert!(!d.dirty());
        d.set_caret(5, false);
        d.input("!").expect("type");
        d.input("!").expect("type");
        assert!(d.dirty());
        assert_eq!(d.buffer.to_text(), "hello!!");
        d.undo().expect("undo");
        assert_eq!(d.buffer.to_text(), "hello");
        d.redo().expect("redo");
        assert_eq!(d.buffer.to_text(), "hello!!");
    }

    #[test]
    fn enter_keeps_indent_and_paste_follows_the_file_eol() {
        let mut d = doc("    fn a() {");
        d.set_caret(12, false);
        d.newline().expect("enter");
        assert_eq!(d.buffer.to_text(), "    fn a() {\n        ");
        let mut crlf = doc("a\r\nb");
        crlf.eol = Eol::CrLf;
        crlf.set_caret(0, false);
        crlf.input("x\ny").expect("paste");
        assert_eq!(crlf.buffer.to_text(), "x\r\nya\r\nb");
    }

    #[test]
    fn delete_handles_crlf_selection_and_edges() {
        let mut d = doc("a\r\nb");
        d.set_caret(3, false);
        d.delete(false, false).expect("backspace over CRLF");
        assert_eq!(d.buffer.to_text(), "ab");
        d.set_caret(0, false);
        d.delete(false, false)
            .expect("backspace at start is a no-op");
        d.select_all();
        d.delete(true, false).expect("delete selection");
        assert_eq!(d.buffer.to_text(), "");
        d.delete(true, false).expect("delete at end is a no-op");
    }

    #[test]
    fn caret_moves_by_word_line_and_keeps_goal_column() {
        let mut d = doc("foo bar\nx\nlonger line");
        d.set_caret(0, false);
        d.move_caret(Move::WordRight, false, 10).expect("word");
        assert_eq!(d.caret, 4);
        d.move_caret(Move::LineEnd, false, 10).expect("end");
        assert_eq!(d.caret, 7);
        d.move_caret(Move::Down, false, 10).expect("down");
        d.move_caret(Move::Down, false, 10).expect("down");
        let pos = d.buffer.char_to_pos(d.caret).expect("pos");
        assert_eq!((pos.line, pos.col), (2, 7));
        d.move_caret(Move::DocStart, true, 10)
            .expect("select to start");
        assert_eq!(d.selection(), (0, d.anchor));
        d.move_caret(Move::Up, false, 10).expect("up at first line");
        assert_eq!(d.caret, 0);
    }

    #[test]
    fn line_commands_follow_the_selection() {
        let mut d = doc("a\nb\nc");
        d.set_caret(0, false);
        d.duplicate_lines().expect("dup");
        assert_eq!(d.buffer.to_text(), "a\na\nb\nc");
        d.move_lines(true).expect("down");
        d.move_lines(true).expect("down again");
        assert_eq!(d.buffer.to_text(), "a\nb\na\nc");
        d.delete_lines().expect("delete line");
        d.toggle_comment().expect("comment");
        assert!(d.buffer.to_text().contains("//"));
        d.undo().expect("undo comment");
    }

    #[test]
    fn read_only_and_loading_refuse_edits() {
        let mut d = doc("x");
        d.read_only = Some("binary".into());
        assert!(d.input("a").is_err());
        d.read_only = None;
        d.phase = Phase::Loading;
        assert!(d.input("a").is_err());
        assert_eq!(d.buffer.to_text(), "x");
    }

    #[test]
    fn goto_line_clamps_and_rejects_zero() {
        let mut d = doc("a\nb\nc");
        assert!(d.goto_line(0).is_err());
        d.goto_line(2).expect("line 2");
        assert_eq!(d.caret, 2);
        d.goto_line(999).expect("clamped to the last line");
        assert_eq!(d.caret, 4);
    }

    #[test]
    fn eol_conversion_is_one_undo_step() {
        let mut d = doc("a\nb\n");
        d.set_eol(Eol::CrLf).expect("crlf");
        assert_eq!(d.buffer.to_text(), "a\r\nb\r\n");
        d.undo().expect("undo");
        assert_eq!(d.buffer.to_text(), "a\nb\n");
    }
}
