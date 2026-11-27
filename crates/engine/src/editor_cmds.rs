//! `editor.*` commands: Edit mode of a Preview pane (capabilities `text-editor` and
//! `editor-open-preference`). Every command but `editor.toggle`, `editor.open`,
//! `editor.open_external`, `editor.window_blur` and `editor.resolve` acts on the focused
//! Preview pane in Edit mode (or an explicit `pane`) and is disabled otherwise.

use std::time::{Duration, Instant};

use fm_editor::{Encoding, Eol, FindOptions, Finder, MAX_MATCHES, Pos, SearchCtl};
use fm_layout::{Axis, PaneId, PaneKind, PreviewSource};
use fm_vfs::{Entry, EntryKind, FmError, VfsUri};
use serde_json::{Map, Value};

use crate::editor_doc::{Move, Pending, Phase};
use crate::editor_hub::check_editable;
use crate::editor_io::{self, EdMsg};
use crate::editor_prefs::{EditorKind, EditorTarget};
use crate::file_cmds::{object, parse_uri};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Text up to this size is searched on the calling thread; larger buffers count their
/// matches on a worker.
const SYNC_FIND_BYTES: usize = 1 << 20;
const FIND_BUDGET: Duration = Duration::from_millis(250);
const REPLACE_BUDGET: Duration = Duration::from_secs(2);
const COUNT_BUDGET: Duration = Duration::from_secs(10);

fn bad(message: impl Into<String>) -> FmError {
    FmError::InvalidParams(message.into())
}

fn no_editor() -> FmError {
    FmError::Unsupported("the focused pane is not editing a file")
}

/// Named params, refusing keys the command does not know.
fn fields<'a>(
    params: &'a Params,
    allowed: &[&str],
) -> Result<Option<&'a Map<String, Value>>, FmError> {
    let map = object(params)?;
    if let Some(map) = map
        && let Some(key) = map.keys().find(|k| !allowed.contains(&k.as_str()))
    {
        return Err(bad(format!("unknown parameter {key:?}")));
    }
    Ok(map)
}

fn opt_bool(map: Option<&Map<String, Value>>, key: &str) -> Result<Option<bool>, FmError> {
    match map.and_then(|m| m.get(key)) {
        None => Ok(None),
        Some(v) => v
            .as_bool()
            .map(Some)
            .ok_or_else(|| bad(format!("{key}: bool"))),
    }
}

fn opt_str<'a>(map: Option<&'a Map<String, Value>>, key: &str) -> Result<Option<&'a str>, FmError> {
    match map.and_then(|m| m.get(key)) {
        None => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| bad(format!("{key}: string"))),
    }
}

fn opt_count(map: Option<&Map<String, Value>>, key: &str) -> Result<Option<usize>, FmError> {
    match map.and_then(|m| m.get(key)) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| bad(format!("{key}: non-negative integer"))),
    }
}

/// The pane a command acts on: `pane` when given, else the focused one.
fn target_pane(s: &Session, map: Option<&Map<String, Value>>) -> Result<PaneId, FmError> {
    match map.and_then(|m| m.get("pane")) {
        None => Ok(s.layout.focused()),
        Some(v) => v
            .as_u64()
            .filter(|p| s.layout.state(*p).is_some() || s.editors.docs.contains_key(p))
            .ok_or_else(|| bad("pane: id of an existing pane")),
    }
}

/// The edited pane a command acts on.
fn edit_pane(s: &Session, map: Option<&Map<String, Value>>) -> Result<PaneId, FmError> {
    let pane = target_pane(s, map)?;
    if s.editors.docs.contains_key(&pane) {
        Ok(pane)
    } else {
        Err(no_editor())
    }
}

/// F4 has something to act on: a Preview pane, or a Files pane with a listed item
/// under the cursor.
fn toggle_enabled(s: &Session) -> bool {
    match s.focused_kind() {
        Some(PaneKind::Preview) => true,
        Some(kind) if kind.is_files() => s.layout.state(s.layout.focused()).is_some_and(|st| {
            !st.listing.on_parent
                && !st.listing.on_current
                && s.dirs.entries(&st.location).is_some_and(|e| !e.is_empty())
        }),
        _ => false,
    }
}

fn editing(s: &Session) -> bool {
    s.editors.docs.contains_key(&s.layout.focused())
}

fn editing_text(s: &Session) -> bool {
    s.editor(s.layout.focused())
        .is_some_and(|d| d.blocked.is_none() && d.load_error.is_none())
}

fn editable_now(s: &Session) -> bool {
    s.editor(s.layout.focused())
        .is_some_and(|d| d.ensure_editable().is_ok())
}

/// Runs `f` on the edited pane's document.
fn with_doc(
    s: &mut Session,
    params: &Params,
    allowed: &[&str],
    f: impl FnOnce(
        &mut crate::editor_doc::EditorDoc,
        Option<&Map<String, Value>>,
    ) -> Result<(), FmError>,
) -> Result<(), FmError> {
    let map = fields(params, allowed)?;
    let pane = edit_pane(s, map)?;
    let doc = s.editor_mut(pane).ok_or_else(no_editor)?;
    f(doc, map)
}

impl Session {
    /// The Preview pane that should edit a file chosen in `origin`: one following
    /// `origin`, else any Preview pane, else a new one split off beside `origin`.
    pub(crate) fn editor_pane_for(&mut self, origin: PaneId) -> Result<PaneId, FmError> {
        let panes = self.layout.panes();
        let is_preview = |s: &Session, p: &PaneId| {
            s.layout
                .state(*p)
                .is_some_and(|st| st.kind == PaneKind::Preview)
        };
        if is_preview(self, &origin) {
            return Ok(origin);
        }
        if let Some(p) = panes
            .iter()
            .find(|p| is_preview(self, p) && self.layout.preview_target(**p) == Some(origin))
        {
            return Ok(*p);
        }
        if let Some(p) = panes.iter().find(|p| is_preview(self, p)) {
            return Ok(*p);
        }
        let from_files = self.layout.state(origin).is_some_and(|o| o.kind.is_files());
        let new_id = self.layout.split(origin, Axis::Horizontal, 0.55)?;
        if let Some(st) = self.layout.state_mut(new_id) {
            st.kind = PaneKind::Preview;
            st.preview_source = if from_files {
                PreviewSource::Pane(origin)
            } else {
                PreviewSource::Follow
            };
            st.edit = None;
        }
        Ok(new_id)
    }

    /// What is known about `uri` without I/O (the cached listing or a preview), else a
    /// bare file entry; the load job finds out the rest.
    fn editor_entry(&self, uri: &VfsUri) -> Entry {
        let cached = uri
            .parent()
            .and_then(|p| self.dirs.entries(&p))
            .and_then(|list| list.iter().find(|e| e.uri == *uri).cloned());
        cached
            .or_else(|| {
                self.previews
                    .slots
                    .values()
                    .filter_map(|slot| slot.subject.as_ref())
                    .find(|e| e.uri == *uri)
                    .cloned()
            })
            .unwrap_or_else(|| Entry::new(uri.clone(), EntryKind::File))
    }

    /// Opens `uri` for editing (F4, `editor.open`, Enter on a text file).
    pub(crate) fn editor_open_uri(
        &mut self,
        uri: VfsUri,
        line: Option<usize>,
        pane: Option<PaneId>,
        activate: bool,
    ) -> Result<(), FmError> {
        let entry = self.editor_entry(&uri);
        check_editable(&entry)?;
        let origin = self.layout.focused();
        let before = self.layout.panes();
        let pane = match pane {
            Some(p) => {
                if self.pane_kind(p) != Some(PaneKind::Preview) {
                    return Err(bad("pane: must be a Preview pane"));
                }
                p
            }
            None => self.editor_pane_for(origin)?,
        };
        self.layout.focus_pane(pane)?;
        self.editors_focus_is(pane);
        let same = self.editor(pane).is_some_and(|d| d.uri == uri);
        if same {
            if let Some(line) = line
                && let Some(doc) = self.editor_mut(pane)
            {
                if doc.phase == Phase::Loading {
                    doc.restore = Some(Pos {
                        line: line.saturating_sub(1),
                        col: 0,
                    });
                } else {
                    doc.goto_line(line)?;
                }
            }
            if let Some(doc) = self.editor_mut(pane) {
                doc.want_focus = true;
            }
            return Ok(());
        }
        if self.editors.docs.contains_key(&pane) {
            self.editor_leave(
                pane,
                Some(Pending::Open {
                    uri,
                    line,
                    activate,
                }),
            );
            return Ok(());
        }
        let caret = line.map(|l| Pos {
            line: l.saturating_sub(1),
            col: 0,
        });
        self.editor_load(pane, uri, caret, 0)?;
        let made_here = !before.contains(&pane);
        if let Some(doc) = self.editor_mut(pane) {
            doc.activated = activate;
            doc.made_for = (activate && made_here).then_some(origin);
        }
        Ok(())
    }

    /// Records that focus is on `pane` now, so moving it there is not a focus loss.
    fn editors_focus_is(&mut self, pane: PaneId) {
        self.editor_focus_set(pane);
    }

    /// Opening an item honours the per-kind preference: the default app for kinds set
    /// to external.
    pub fn editor_target_for(&self, uri: &VfsUri) -> Option<EditorTarget> {
        EditorKind::of_uri(uri).map(|k| self.prefs.editor.open.get(k))
    }

    /// The focused Files pane's item under the cursor, as a file.
    fn cursor_file(&mut self) -> Result<Entry, FmError> {
        let pane = self.layout.focused();
        let entry = self
            .cursor_entry(pane)
            .ok_or_else(|| bad("select a file to edit"))?;
        if entry.is_dir_like() {
            return Err(bad("folders cannot be edited"));
        }
        Ok(entry)
    }

    /// Text typed or pasted into the editor of `pane`. Not limited by the command
    /// parameter size, so large pastes go through here.
    pub fn editor_input_text(&mut self, pane: PaneId, text: &str) -> Result<(), FmError> {
        let doc = self.editor_mut(pane).ok_or_else(no_editor)?;
        doc.input(text)?;
        self.editor_find_stale(pane);
        Ok(())
    }

    /// Recounts find matches after the text or the query changed.
    pub(crate) fn editor_find_refresh(&mut self, pane: PaneId) {
        let Some(doc) = self.editors.docs.get_mut(&pane) else {
            return;
        };
        doc.find.rev += 1;
        doc.find.matches.clear();
        doc.find.truncated = false;
        doc.find.error = None;
        doc.find.busy = false;
        if !doc.find.open || doc.find.query.is_empty() {
            return;
        }
        let finder = match Finder::new(&doc.find.query, doc.find.options) {
            Ok(f) => f,
            Err(e) => {
                doc.find.error = Some(e.to_string());
                return;
            }
        };
        doc.find.version = doc.buffer.version();
        if doc.buffer.len_bytes() <= SYNC_FIND_BYTES {
            let ctl = SearchCtl {
                cancel: None,
                deadline: Some(Instant::now() + FIND_BUDGET),
            };
            match finder.find_all(&doc.buffer, MAX_MATCHES, &ctl) {
                Ok((matches, truncated)) => {
                    doc.find.matches = matches;
                    doc.find.truncated = truncated;
                }
                Err(e) => doc.find.error = Some(e.to_string()),
            }
            return;
        }
        doc.find.busy = true;
        let buffer = doc.buffer.clone();
        let (rev, token, version) = (doc.find.rev, doc.token, doc.buffer.version());
        let cancel = doc.parse_ctl.cancel.clone();
        let inbox = self.editors.inbox.clone();
        let _ = std::thread::Builder::new()
            .name("fm-find".into())
            .spawn(move || {
                let ctl = SearchCtl {
                    cancel: Some(&cancel),
                    deadline: Some(Instant::now() + COUNT_BUDGET),
                };
                let result = finder
                    .find_all(&buffer, MAX_MATCHES, &ctl)
                    .map_err(|e| e.to_string());
                editor_io::push(
                    &inbox,
                    EdMsg::Found {
                        pane,
                        token,
                        rev,
                        version,
                        result,
                    },
                );
            });
    }

    /// The text changed: an open find bar recounts.
    pub(crate) fn editor_find_stale(&mut self, pane: PaneId) {
        if self
            .editor(pane)
            .is_some_and(|d| d.find.open && !d.find.query.is_empty())
        {
            self.editor_find_refresh(pane);
        }
    }

    /// Moves to the next (or previous) match and selects it.
    fn editor_find_step(&mut self, pane: PaneId, backward: bool) -> Result<(), FmError> {
        let doc = self.editor_mut(pane).ok_or_else(no_editor)?;
        if doc.find.query.is_empty() {
            doc.find.open = true;
            return Ok(());
        }
        let finder =
            Finder::new(&doc.find.query, doc.find.options).map_err(|e| bad(e.to_string()))?;
        let ctl = SearchCtl {
            cancel: None,
            deadline: Some(Instant::now() + FIND_BUDGET),
        };
        let (a, b) = doc.selection();
        let found = if backward {
            finder.find_prev(&doc.buffer, a, &ctl)
        } else {
            let from = if a == b { b } else { a + 1 };
            finder.find_next(&doc.buffer, from, &ctl)
        }
        .map_err(|e| bad(e.to_string()))?;
        match found {
            Some(m) => {
                doc.set_caret(m.start, false);
                doc.set_caret(m.end, true);
                doc.reveal = true;
                Ok(())
            }
            None => Err(FmError::NotFound(format!("\"{}\"", doc.find.query))),
        }
    }

    fn editor_replace(&mut self, pane: PaneId, all: bool) -> Result<(), FmError> {
        let doc = self.editor_mut(pane).ok_or_else(no_editor)?;
        doc.ensure_editable()?;
        let finder =
            Finder::new(&doc.find.query, doc.find.options).map_err(|e| bad(e.to_string()))?;
        let replacement = doc.find.replacement.clone();
        if all {
            let ctl = SearchCtl {
                cancel: None,
                deadline: Some(Instant::now() + REPLACE_BUDGET),
            };
            let recs = finder
                .replace_all(&mut doc.buffer, &replacement, &ctl)
                .map_err(|e| bad(e.to_string()))?;
            doc.replaced(recs);
        } else {
            let selected = doc.selected_text().unwrap_or_default();
            match finder.expand(&selected, &replacement) {
                Some(text) if !selected.is_empty() => doc.input(&text)?,
                _ => {}
            }
            self.editor_find_step(pane, false).ok();
        }
        self.editor_find_stale(pane);
        Ok(())
    }

    pub(crate) fn editor_found(
        &mut self,
        pane: PaneId,
        token: u64,
        rev: u64,
        version: u64,
        result: Result<(Vec<fm_editor::Match>, bool), String>,
    ) {
        let Some(doc) = self
            .editors
            .docs
            .get_mut(&pane)
            .filter(|d| d.token == token && d.find.rev == rev)
        else {
            return;
        };
        doc.find.busy = false;
        doc.find.version = version;
        match result {
            Ok((matches, truncated)) => {
                doc.find.matches = matches;
                doc.find.truncated = truncated;
            }
            Err(e) => doc.find.error = Some(e),
        }
    }
}

fn parse_encoding(name: &str) -> Option<Encoding> {
    Some(match name {
        "utf-8" => Encoding::Utf8,
        "utf-8-bom" => Encoding::Utf8Bom,
        "utf-16le" => Encoding::Utf16Le,
        "utf-16be" => Encoding::Utf16Be,
        "latin-1" => Encoding::Latin1,
        _ => return None,
    })
}

/// `120`, `120:8` or `120,8` from the jump-to-line prompt.
pub fn parse_goto(text: &str) -> Result<(usize, Option<usize>), FmError> {
    let text = text.trim();
    let (line, col) = match text.split_once([':', ',']) {
        Some((l, c)) => (l.trim(), Some(c.trim())),
        None => (text, None),
    };
    let num = |s: &str| {
        s.parse::<usize>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| bad("type a line number from 1, optionally :column"))
    };
    Ok((num(line)?, col.map(num).transpose()?))
}

/// One `editor.input` operation.
fn input_op(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params)?.ok_or_else(|| bad("expected {op, …}"))?;
    let pane = edit_pane(s, Some(map))?;
    let op = map
        .get("op")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("op: string required"))?;
    let allowed: &[&str] = match op {
        "insert" => &["op", "pane", "text"],
        "backspace" | "delete" => &["op", "pane", "word"],
        "move" => &["op", "pane", "to", "extend", "page"],
        "click" => &["op", "pane", "line", "col", "extend"],
        "select_word" => &["op", "pane", "line", "col"],
        "move_lines" => &["op", "pane", "down"],
        "newline" | "select_all" | "indent" | "outdent" | "duplicate_lines" | "delete_lines"
        | "toggle_comment" | "cut" => &["op", "pane"],
        other => return Err(bad(format!("unknown op {other:?}"))),
    };
    let map = fields(params, allowed)?;
    let edits_text = !matches!(op, "move" | "click" | "select_word" | "select_all");
    let doc = s.editor_mut(pane).ok_or_else(no_editor)?;
    match op {
        "insert" => {
            let text = opt_str(map, "text")?.ok_or_else(|| bad("text: string required"))?;
            doc.input(text)?;
        }
        "newline" => doc.newline()?,
        "backspace" => doc.delete(false, opt_bool(map, "word")?.unwrap_or(false))?,
        "delete" => doc.delete(true, opt_bool(map, "word")?.unwrap_or(false))?,
        "move" => {
            let to = opt_str(map, "to")?.ok_or_else(|| bad("to: string required"))?;
            let mv = Move::parse(to).ok_or_else(|| bad(format!("unknown move {to:?}")))?;
            let extend = opt_bool(map, "extend")?.unwrap_or(false);
            let page = opt_count(map, "page")?.unwrap_or(20).clamp(1, 10_000);
            doc.move_caret(mv, extend, page)?;
        }
        "click" | "select_word" => {
            let line = opt_count(map, "line")?.ok_or_else(|| bad("line: integer required"))?;
            let col = opt_count(map, "col")?.unwrap_or(0);
            let last = doc.buffer.len_lines().saturating_sub(1);
            let line = line.min(last);
            let col = col.min(doc.buffer.line_len(line).unwrap_or(0));
            let at = doc
                .buffer
                .pos_to_char(Pos { line, col })
                .map_err(|e| bad(e.to_string()))?;
            if op == "click" {
                doc.set_caret(at, opt_bool(map, "extend")?.unwrap_or(false));
            } else {
                doc.select_word(at);
            }
        }
        "select_all" => doc.select_all(),
        "indent" => doc.indent_selection(false)?,
        "outdent" => doc.indent_selection(true)?,
        "duplicate_lines" => doc.duplicate_lines()?,
        "delete_lines" => doc.delete_lines()?,
        "move_lines" => doc.move_lines(opt_bool(map, "down")?.unwrap_or(true))?,
        "toggle_comment" => doc.toggle_comment()?,
        "cut" if doc.has_selection() => doc.delete(true, false)?,
        _ => {}
    }
    if edits_text {
        s.editor_find_stale(pane);
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        CommandSpec {
            id: "editor.toggle",
            label: "Edit File",
            menu: &["File", "Edit File"],
            shortcut: Some("F4"),
            params: "{pane?: number}: Files pane: edit the item under the cursor; Preview pane: switch between preview and Edit mode",
            enabled: toggle_enabled,
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = target_pane(s, map)?;
                match s.pane_kind(pane) {
                    Some(PaneKind::Preview) if s.editors.docs.contains_key(&pane) => {
                        s.editor_leave(pane, Some(Pending::Leave));
                        Ok(())
                    }
                    Some(PaneKind::Preview) => {
                        let entry = s
                            .preview_subject(pane)
                            .ok_or_else(|| bad("the preview shows no single file to edit"))?;
                        check_editable(&entry)?;
                        if let Some(crate::preview_cmds::Body::Static(doc)) =
                            s.previews.slot(pane).map(|sl| &sl.body)
                            && matches!(
                                &**doc,
                                fm_preview::Preview::Image(_)
                                    | fm_preview::Preview::Pdf(_)
                                    | fm_preview::Preview::Dir(_)
                                    | fm_preview::Preview::Hex(_)
                                    | fm_preview::Preview::Meta(_)
                            )
                        {
                            return Err(bad("this type of file cannot be edited"));
                        }
                        s.editor_load(pane, entry.uri, None, 0)
                    }
                    Some(kind) if kind.is_files() => {
                        let entry = s.cursor_file()?;
                        if s.editor_target_for(&entry.uri) == Some(EditorTarget::External) {
                            let params = serde_json::json!({ "uri": entry.uri.to_string() });
                            return s.dispatch("file.open", &params);
                        }
                        s.editor_open_uri(entry.uri, None, None, false)
                    }
                    _ => Err(bad("F4 edits a file from a Files or Preview pane")),
                }
            },
        },
        CommandSpec {
            id: "editor.open",
            label: "Open in Editor",
            menu: &["File", "Editor", "Open in Editor"],
            shortcut: None,
            params: "{uri: string, line?: number from 1, pane?: number, activate?: bool}: open a file in Edit mode of a Preview pane; activate (Enter/double-click) sends a file that turns out binary to its default application instead",
            enabled: |_| true,
            run: |s, p| {
                let map = fields(p, &["uri", "line", "pane", "activate"])?;
                let uri = map
                    .and_then(|m| m.get("uri"))
                    .ok_or_else(|| bad("uri: string required"))
                    .and_then(parse_uri)?;
                let line = opt_count(map, "line")?;
                if line == Some(0) {
                    return Err(bad("line: lines start at 1"));
                }
                let pane = match map.and_then(|m| m.get("pane")) {
                    None => None,
                    Some(_) => Some(target_pane(s, map)?),
                };
                let activate = match map.and_then(|m| m.get("activate")) {
                    None => false,
                    Some(v) => v.as_bool().ok_or_else(|| bad("activate: true or false"))?,
                };
                s.editor_open_uri(uri, line, pane, activate)
            },
        },
        CommandSpec {
            id: "editor.save",
            label: "Save",
            menu: &["File", "Save"],
            shortcut: Some("Ctrl+S"),
            params: "{pane?: number}",
            enabled: |s| {
                s.editor(s.layout.focused())
                    .is_some_and(|d| d.savable() && d.dirty())
            },
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_save(pane, None, true)
            },
        },
        CommandSpec {
            id: "editor.save_as",
            label: "Save As…",
            menu: &["File", "Editor", "Save As…"],
            shortcut: None,
            params: "{uri: string, overwrite?: bool, pane?: number}: write the text to another file and edit that one",
            enabled: editing_text,
            run: |s, p| {
                let map = fields(p, &["uri", "overwrite", "pane"])?;
                let pane = edit_pane(s, map)?;
                let uri = map
                    .and_then(|m| m.get("uri"))
                    .ok_or_else(|| bad("uri: string required"))
                    .and_then(parse_uri)?;
                if uri.file_name().is_none() {
                    return Err(bad("uri: must name a file"));
                }
                let overwrite = opt_bool(map, "overwrite")?.unwrap_or(false);
                if let Some(doc) = s.editor_mut(pane) {
                    doc.conflict = None;
                }
                s.editor_save_to(pane, Some(uri), false, !overwrite)
            },
        },
        CommandSpec {
            id: "editor.revert",
            label: "Revert to Saved",
            menu: &["File", "Editor", "Revert to Saved"],
            shortcut: None,
            params: "{pane?: number}: drop unsaved edits and read the file again",
            enabled: |s| s.editor(s.layout.focused()).is_some_and(|d| d.dirty()),
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_reload(pane)
            },
        },
        CommandSpec {
            id: "editor.goto_prompt",
            label: "Go to Line…",
            menu: &["File", "Editor", "Go to Line…"],
            shortcut: Some("Ctrl+G"),
            params: "{pane?: number, close?: bool}: show (or close) the jump-to-line prompt",
            enabled: editing_text,
            run: |s, p| {
                with_doc(s, p, &["pane", "close"], |doc, map| {
                    let close = opt_bool(map, "close")?.unwrap_or(false);
                    doc.goto_open = !close;
                    if close {
                        doc.want_focus = true;
                    }
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.goto_line",
            label: "Go to Line",
            menu: &["File", "Editor", "Go to Line"],
            shortcut: None,
            params: "{line: number from 1, column?: number from 1, pane?: number} or {text: \"120:8\"}",
            enabled: editing_text,
            run: |s, p| {
                with_doc(s, p, &["line", "column", "text", "pane"], |doc, map| {
                    let (line, col) = match (opt_count(map, "line")?, opt_str(map, "text")?) {
                        (Some(line), None) => (line, opt_count(map, "column")?),
                        (None, Some(text)) => parse_goto(text)?,
                        _ => return Err(bad("line: number or text: \"line:column\" required")),
                    };
                    if col == Some(0) {
                        return Err(bad("column: columns start at 1"));
                    }
                    doc.goto_line(line)?;
                    if let Some(col) = col {
                        let pos = doc.buffer.char_to_pos(doc.caret).unwrap_or_default();
                        let len = doc.buffer.line_len(pos.line).unwrap_or(0);
                        let at = doc
                            .buffer
                            .pos_to_char(Pos {
                                line: pos.line,
                                col: (col - 1).min(len),
                            })
                            .map_err(|e| bad(e.to_string()))?;
                        doc.set_caret(at, false);
                    }
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.find",
            label: "Find in File",
            menu: &["File", "Editor", "Find in File"],
            shortcut: Some("Ctrl+F"),
            params: "{query?: string, replacement?: string, case?: bool, word?: bool, regex?: bool, replace?: bool, pane?: number}: open the find bar and set its fields",
            enabled: editing_text,
            run: |s, p| {
                let keys = [
                    "query",
                    "replacement",
                    "case",
                    "word",
                    "regex",
                    "replace",
                    "pane",
                ];
                let map = fields(p, &keys)?;
                let pane = edit_pane(s, map)?;
                let query = opt_str(map, "query")?;
                let replacement = opt_str(map, "replacement")?;
                if query.is_some_and(|q| q.chars().count() > fm_editor::MAX_QUERY_CHARS)
                    || replacement
                        .is_some_and(|r| r.chars().count() > fm_editor::MAX_REPLACEMENT_CHARS)
                {
                    return Err(bad("the search text or replacement is too long"));
                }
                let (case, word, regex, replace) = (
                    opt_bool(map, "case")?,
                    opt_bool(map, "word")?,
                    opt_bool(map, "regex")?,
                    opt_bool(map, "replace")?,
                );
                let doc = s.editor_mut(pane).ok_or_else(no_editor)?;
                let first_open = !doc.find.open;
                doc.find.open = true;
                if let Some(r) = replace {
                    doc.find.replace = r;
                }
                if let Some(q) = query {
                    doc.find.query = q.to_string();
                } else if first_open
                    && let Some(sel) = doc.selected_text()
                    && !sel.contains('\n')
                    && sel.chars().count() <= fm_editor::MAX_QUERY_CHARS
                {
                    doc.find.query = sel;
                }
                if let Some(r) = replacement {
                    doc.find.replacement = r.to_string();
                }
                let opts: &mut FindOptions = &mut doc.find.options;
                if let Some(c) = case {
                    opts.case_sensitive = c;
                }
                if let Some(w) = word {
                    opts.whole_word = w;
                }
                if let Some(r) = regex {
                    opts.regex = r;
                }
                s.editor_find_refresh(pane);
                Ok(())
            },
        },
        CommandSpec {
            id: "editor.find_next",
            label: "Find Next in File",
            menu: &["File", "Editor", "Find Next in File"],
            shortcut: Some("F3"),
            params: "{pane?: number}",
            enabled: editing_text,
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_find_step(pane, false)
            },
        },
        CommandSpec {
            id: "editor.find_previous",
            label: "Find Previous in File",
            menu: &["File", "Editor", "Find Previous in File"],
            shortcut: Some("Shift+F3"),
            params: "{pane?: number}",
            enabled: editing_text,
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_find_step(pane, true)
            },
        },
        CommandSpec {
            id: "editor.replace",
            label: "Replace",
            menu: &["File", "Editor", "Replace"],
            shortcut: None,
            params: "{all?: bool, pane?: number}: replace the selected match (and find the next) or every match as one undo step",
            enabled: editable_now,
            run: |s, p| {
                let map = fields(p, &["all", "pane"])?;
                let pane = edit_pane(s, map)?;
                let all = opt_bool(map, "all")?.unwrap_or(false);
                s.editor_replace(pane, all)
            },
        },
        CommandSpec {
            id: "editor.find_close",
            label: "Close Find",
            menu: &["File", "Editor", "Close Find"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: editing,
            run: |s, p| {
                with_doc(s, p, &["pane"], |doc, _| {
                    doc.find.open = false;
                    doc.find.matches.clear();
                    doc.want_focus = true;
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.undo",
            label: "Undo Typing",
            menu: &["File", "Editor", "Undo Typing"],
            shortcut: Some("Ctrl+Z"),
            params: "{pane?: number}",
            enabled: |s| {
                s.editor(s.layout.focused())
                    .is_some_and(|d| d.undo.can_undo())
            },
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_mut(pane).ok_or_else(no_editor)?.undo()?;
                s.editor_find_stale(pane);
                Ok(())
            },
        },
        CommandSpec {
            id: "editor.redo",
            label: "Redo Typing",
            menu: &["File", "Editor", "Redo Typing"],
            shortcut: Some("Ctrl+Y"),
            params: "{pane?: number}",
            enabled: |s| {
                s.editor(s.layout.focused())
                    .is_some_and(|d| d.undo.can_redo())
            },
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                s.editor_mut(pane).ok_or_else(no_editor)?.redo()?;
                s.editor_find_stale(pane);
                Ok(())
            },
        },
        CommandSpec {
            id: "editor.input",
            label: "Editor Input",
            menu: &["File", "Editor", "Editor Input"],
            shortcut: None,
            params: "{op: \"insert\" {text} | \"newline\" | \"backspace\"/\"delete\" {word?} | \"move\" {to, extend?, page?} | \"click\" {line, col?, extend?} | \"select_word\" {line, col?} | \"select_all\" | \"indent\" | \"outdent\" | \"duplicate_lines\" | \"delete_lines\" | \"move_lines\" {down?} | \"toggle_comment\" | \"cut\", pane?: number}",
            enabled: editing_text,
            run: input_op,
        },
        CommandSpec {
            id: "editor.set_encoding",
            label: "File Encoding",
            menu: &["File", "Editor", "File Encoding"],
            shortcut: None,
            params: "{encoding: \"utf-8\"|\"utf-8-bom\"|\"utf-16le\"|\"utf-16be\"|\"latin-1\", pane?: number}: the encoding the next save writes",
            enabled: editable_now,
            run: |s, p| {
                with_doc(s, p, &["encoding", "pane"], |doc, map| {
                    let name = opt_str(map, "encoding")?
                        .ok_or_else(|| bad("encoding: string required"))?;
                    let enc = parse_encoding(name)
                        .ok_or_else(|| bad(format!("unknown encoding {name:?}")))?;
                    doc.ensure_editable()?;
                    if enc == Encoding::Latin1
                        && doc
                            .buffer
                            .chunks()
                            .any(|c| c.chars().any(|ch| u32::from(ch) > 0xFF))
                    {
                        return Err(bad("the text has characters Latin-1 cannot hold"));
                    }
                    if doc.encoding != enc {
                        doc.encoding = enc;
                        // The bytes on disk will differ: the buffer counts as changed.
                        doc.clean_version = doc.clean_version.wrapping_sub(1);
                    }
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.set_eol",
            label: "Line Endings",
            menu: &["File", "Editor", "Line Endings"],
            shortcut: None,
            params: "{eol: \"lf\"|\"crlf\", pane?: number}: rewrite every line ending (one undo step)",
            enabled: editable_now,
            run: |s, p| {
                with_doc(s, p, &["eol", "pane"], |doc, map| {
                    let eol = match opt_str(map, "eol")? {
                        Some("lf") => Eol::Lf,
                        Some("crlf") => Eol::CrLf,
                        _ => return Err(bad("eol: \"lf\" or \"crlf\" required")),
                    };
                    doc.set_eol(eol)
                })
            },
        },
        CommandSpec {
            id: "editor.toggle_wrap",
            label: "Wrap Lines in Editor",
            menu: &["File", "Editor", "Wrap Lines in Editor"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: editing_text,
            run: |s, p| {
                with_doc(s, p, &["pane"], |doc, _| {
                    doc.wrap = !doc.wrap;
                    doc.reveal = true;
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.toggle_line_numbers",
            label: "Line Numbers",
            menu: &["File", "Editor", "Line Numbers"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: editing_text,
            run: |s, p| {
                with_doc(s, p, &["pane"], |doc, _| {
                    doc.line_numbers = !doc.line_numbers;
                    Ok(())
                })
            },
        },
        CommandSpec {
            id: "editor.open_external",
            label: "Open in External Editor",
            menu: &["File", "Open in External Editor"],
            shortcut: None,
            params: "{uri?: string, pane?: number}: the file of the editor (saved first) or the item under the cursor, in its default application",
            enabled: |s| s.editors.docs.contains_key(&s.layout.focused()) || s.focused_is_files(),
            run: |s, p| {
                let map = fields(p, &["uri", "pane"])?;
                if let Some(uri) = map.and_then(|m| m.get("uri")) {
                    let uri = parse_uri(uri)?;
                    let params = serde_json::json!({ "uri": uri.to_string() });
                    return s.dispatch("file.open", &params);
                }
                let pane = target_pane(s, map)?;
                if s.editors.docs.contains_key(&pane) {
                    s.editor_leave(pane, Some(Pending::External));
                    return Ok(());
                }
                let entry = s.cursor_file()?;
                let params = serde_json::json!({ "uri": entry.uri.to_string() });
                s.dispatch("file.open", &params)
            },
        },
        CommandSpec {
            id: "editor.window_blur",
            label: "Window Lost Focus",
            menu: &["File", "Editor", "Window Lost Focus"],
            shortcut: None,
            params: "none: the application window lost focus (autosave trigger)",
            enabled: |_| true,
            run: |s, p| {
                fields(p, &[])?;
                s.editor_window_blur();
                Ok(())
            },
        },
        CommandSpec {
            id: "editor.resolve",
            label: "Editor Decision",
            menu: &["File", "Editor", "Editor Decision"],
            shortcut: None,
            params: "{choice: \"save\"|\"discard\"|\"cancel\"|\"reload\"|\"overwrite\"|\"keep\", pane?: number}: answer the unsaved-changes warning, the conflict prompt or the changed-on-disk bar",
            enabled: |s| {
                s.editors
                    .docs
                    .values()
                    .any(|d| d.leave_prompt || d.conflict.is_some() || d.changed_on_disk)
            },
            run: |s, p| {
                let map = fields(p, &["choice", "pane"])?;
                let choice =
                    opt_str(map, "choice")?.ok_or_else(|| bad("choice: string required"))?;
                if !matches!(
                    choice,
                    "save" | "discard" | "cancel" | "reload" | "overwrite" | "keep"
                ) {
                    return Err(bad(format!("unknown choice {choice:?}")));
                }
                let pane = match map.and_then(|m| m.get("pane")) {
                    Some(_) => edit_pane(s, map)?,
                    None => s
                        .editor_asking()
                        .ok_or(FmError::Unsupported("no editor is waiting for a decision"))?,
                };
                s.editor_resolve(pane, choice)
            },
        },
    ];
    v.extend(crate::editor_prefs::specs());
    v.extend(crate::editor_fold_cmds::specs());
    v.extend(crate::editor_live::specs());
    v
}

impl Session {
    /// The editor waiting for a decision: the focused one first.
    pub fn editor_asking(&self) -> Option<PaneId> {
        let asks = |d: &crate::editor_doc::EditorDoc| {
            d.leave_prompt || d.conflict.is_some() || d.changed_on_disk
        };
        let focused = self.layout.focused();
        if self.editor(focused).is_some_and(asks) {
            return Some(focused);
        }
        let mut panes: Vec<PaneId> = self
            .editors
            .docs
            .iter()
            .filter(|(_, d)| asks(d))
            .map(|(p, _)| *p)
            .collect();
        panes.sort_unstable();
        panes.first().copied()
    }
}
