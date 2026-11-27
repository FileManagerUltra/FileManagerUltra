//! Folding commands of the editor (change `add-editor-folding-live-preview`):
//! `editor.fold`, `editor.unfold`, `editor.toggle_fold`, `editor.fold_all`,
//! `editor.unfold_all`, and the rainbow-bracket preference. Folds are view state of one
//! editor: the text never changes.

use fm_layout::PaneId;
use fm_vfs::FmError;
use serde_json::{Map, Value};

use crate::editor_doc::EditorDoc;
use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Lines `fold_all` walks at most without a syntax tree (indentation folds).
const FOLD_ALL_SCAN: usize = 1_000_000;
/// How far above the caret a containing fold is looked for.
const ENCLOSING_SCAN: usize = 10_000;

fn bad(message: impl Into<String>) -> FmError {
    FmError::InvalidParams(message.into())
}

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

fn edit_pane(s: &Session, map: Option<&Map<String, Value>>) -> Result<PaneId, FmError> {
    let pane = match map.and_then(|m| m.get("pane")) {
        None => s.layout.focused(),
        Some(v) => v.as_u64().ok_or_else(|| bad("pane: number"))?,
    };
    if s.editors.docs.contains_key(&pane) {
        Ok(pane)
    } else {
        Err(FmError::Unsupported(
            "the focused pane is not editing a file",
        ))
    }
}

/// The 0-based line a command acts on: `line` (from 1) or the caret's line.
fn target_line(doc: &EditorDoc, map: Option<&Map<String, Value>>) -> Result<usize, FmError> {
    match map.and_then(|m| m.get("line")) {
        None => Ok(doc.buffer.char_to_pos(doc.caret).map_or(0, |p| p.line)),
        Some(v) => {
            let n = v
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| bad("line: number from 1"))?;
            let last = doc.buffer.len_lines();
            if n > last {
                return Err(bad(format!("line: the file has {last} lines")));
            }
            Ok(n - 1)
        }
    }
}

impl EditorDoc {
    /// The foldable range at `line`, or the innermost one above that contains it.
    pub fn fold_range_around(&self, line: usize) -> Option<(usize, usize)> {
        if let Some(end) = self.fold_ranges.range_at(&self.buffer, line) {
            return Some((line, end));
        }
        let floor = line.saturating_sub(ENCLOSING_SCAN);
        (floor..line)
            .rev()
            .filter(|l| !self.folds.is_hidden(*l))
            .find_map(|start| {
                self.fold_ranges
                    .range_at(&self.buffer, start)
                    .filter(|end| *end >= line)
                    .map(|end| (start, end))
            })
    }

    /// Folds the range at (or around) `line`; the caret leaves the hidden lines.
    pub fn fold_line(&mut self, line: usize) -> Result<bool, FmError> {
        let Some((start, end)) = self.fold_range_around(line) else {
            return Ok(false);
        };
        let done = self.folds.fold(start, end);
        if done {
            self.caret_out_of_folds();
        }
        Ok(done)
    }

    /// Unfolds the fold at `line` or hiding it.
    pub fn unfold_line(&mut self, line: usize) -> bool {
        if self.folds.folded_at(line).is_some() {
            return self.folds.unfold(line);
        }
        self.folds.reveal(line)
    }

    pub fn toggle_fold_line(&mut self, line: usize) -> Result<bool, FmError> {
        if self.folds.folded_at(line).is_some() {
            return Ok(self.folds.unfold(line));
        }
        self.fold_line(line)
    }

    /// Folds every outermost range.
    pub fn fold_all(&mut self) {
        let total = self.buffer.len_lines();
        let mut line = 0usize;
        let limit = total.min(FOLD_ALL_SCAN);
        while line < limit {
            match self.fold_ranges.range_at(&self.buffer, line) {
                Some(end) => {
                    self.folds.fold(line, end);
                    line = end + 1;
                }
                None => line += 1,
            }
        }
        self.caret_out_of_folds();
    }

    /// A caret in hidden lines moves to the start of the fold's first line.
    pub(crate) fn caret_out_of_folds(&mut self) {
        let Ok(pos) = self.buffer.char_to_pos(self.caret) else {
            return;
        };
        if let Some((start, _)) = self.folds.hiding(pos.line)
            && let Ok(at) = self.buffer.line_start(start)
        {
            self.set_caret(at, false);
        }
        let anchor_line = self.buffer.char_to_pos(self.anchor).map_or(0, |p| p.line);
        if self.folds.is_hidden(anchor_line) {
            self.anchor = self.caret;
        }
        self.reveal = true;
    }
}

fn line_cmd(
    s: &mut Session,
    params: &Params,
    f: impl FnOnce(&mut EditorDoc, usize) -> Result<bool, FmError>,
) -> Result<(), FmError> {
    let map = fields(params, &["line", "pane"])?;
    let pane = edit_pane(s, map)?;
    let doc = s.editor_mut(pane).ok_or(FmError::Unsupported(
        "the focused pane is not editing a file",
    ))?;
    let line = target_line(doc, map)?;
    f(doc, line).map(|_| ())
}

fn editing(s: &Session) -> bool {
    s.editor(s.layout.focused())
        .is_some_and(|d| d.blocked.is_none() && d.load_error.is_none())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "editor.fold",
            label: "Fold",
            menu: &["File", "Editor", "Fold"],
            shortcut: Some("Ctrl+Shift+["),
            params: "{line?: number from 1, pane?: number}: fold the block at the line (default: the caret's)",
            enabled: editing,
            run: |s, p| line_cmd(s, p, EditorDoc::fold_line),
        },
        CommandSpec {
            id: "editor.unfold",
            label: "Unfold",
            menu: &["File", "Editor", "Unfold"],
            shortcut: Some("Ctrl+Shift+]"),
            params: "{line?: number from 1, pane?: number}: unfold the block at or hiding the line",
            enabled: editing,
            run: |s, p| line_cmd(s, p, |doc, line| Ok(doc.unfold_line(line))),
        },
        CommandSpec {
            id: "editor.toggle_fold",
            label: "Toggle Fold",
            menu: &["File", "Editor", "Toggle Fold"],
            shortcut: None,
            params: "{line?: number from 1, pane?: number}: fold or unfold the block at the line (the gutter chevron)",
            enabled: editing,
            run: |s, p| line_cmd(s, p, EditorDoc::toggle_fold_line),
        },
        CommandSpec {
            id: "editor.fold_all",
            label: "Fold All",
            menu: &["File", "Editor", "Fold All"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: editing,
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                if let Some(doc) = s.editor_mut(pane) {
                    doc.fold_all();
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "editor.unfold_all",
            label: "Unfold All",
            menu: &["File", "Editor", "Unfold All"],
            shortcut: None,
            params: "{pane?: number}",
            enabled: |s| {
                s.editor(s.layout.focused())
                    .is_some_and(|d| !d.folds.is_empty())
            },
            run: |s, p| {
                let map = fields(p, &["pane"])?;
                let pane = edit_pane(s, map)?;
                if let Some(doc) = s.editor_mut(pane) {
                    doc.folds.clear();
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "prefs.editor_rainbow",
            label: "Rainbow Brackets",
            menu: &["Edit", "Preferences", "Rainbow brackets"],
            shortcut: None,
            params: "{enabled: bool}: colour brackets by nesting depth",
            enabled: |_| true,
            run: |s, p| {
                let v = crate::editor_prefs::one_field(p, "enabled", "bool")?;
                s.prefs.editor.rainbow = v.as_bool().ok_or_else(|| bad("enabled: bool"))?;
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use crate::editor_doc::Phase;
    use fm_vfs::VfsUri;

    fn doc(text: &str) -> EditorDoc {
        let mut d = EditorDoc::new(VfsUri::parse("/tmp/a.py").expect("uri"), 100);
        d.buffer.append(text);
        d.clean_version = d.buffer.version();
        d.phase = Phase::Ready;
        d
    }

    #[test]
    fn indentation_folds_follow_the_caret() {
        let mut d = doc("def f():\n    a = 1\n    b = 2\nprint(f())\n");
        d.goto_line(3).expect("line 3");
        assert!(d.fold_line(2).expect("fold"), "the enclosing block folds");
        assert_eq!(d.folds.ranges(), &[(0, 2)]);
        assert_eq!(d.caret, 0, "the caret left the hidden lines");
        assert!(d.toggle_fold_line(0).expect("toggle"));
        assert!(d.folds.is_empty());
        d.fold_all();
        assert_eq!(d.folds.ranges(), &[(0, 2)]);
        assert!(d.unfold_line(1));
    }
}
