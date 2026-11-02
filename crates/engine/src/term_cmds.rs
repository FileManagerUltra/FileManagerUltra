//! Terminal panes: session registry on `Session` plus the `term.*` commands.
//!
//! The UI reads snapshots and sends keystrokes through `term.input`; it never touches the
//! pty. Selection and clipboard hand-off live here so the commands stay testable.

use fm_layout::{PaneId, PaneKind};
use fm_term::{Pump, Snapshot, Status, TermSession};
use fm_vfs::{FmError, VfsUri};

use crate::file_cmds::{object, other_pane_dir};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Largest keystroke batch `term.input` accepts (pastes use `term.paste`).
pub const MAX_KEYS_BYTES: usize = 64 * 1024;
/// Largest paste accepted.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;
const DEFAULT_ROWS: u16 = 24;
const DEFAULT_COLS: u16 = 80;

/// A text selection in retained buffer cells (history followed by the live screen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSelection {
    pub pane: PaneId,
    pub anchor: (usize, u16),
    pub head: (usize, u16),
}

fn local_dir(uri: &VfsUri) -> Option<String> {
    uri.local_path_string()
        .filter(|p| std::path::Path::new(p).is_dir())
}

/// The native path of a local file or folder uri, whether or not it exists.
fn local_path(uri: &VfsUri) -> Option<String> {
    uri.local_path_string()
}

fn home_dir() -> Option<String> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_string_lossy().into_owned())
}

impl Session {
    pub fn pane_kind(&self, pane: PaneId) -> Option<PaneKind> {
        self.layout.state(pane).map(|p| p.kind)
    }

    pub fn focused_kind(&self) -> Option<PaneKind> {
        self.pane_kind(self.layout.focused())
    }

    /// The focused pane shows files (the default for every file/nav command).
    pub fn focused_is_files(&self) -> bool {
        self.focused_kind().is_some_and(PaneKind::is_files)
    }

    pub fn terminal(&self, pane: PaneId) -> Option<&TermSession> {
        self.terminals.get(&pane)
    }

    pub fn terminal_snapshot(&self, pane: PaneId) -> Option<Snapshot> {
        self.terminals.get(&pane).map(TermSession::snapshot)
    }

    /// Start the pane's shell if it has none: in the last active Files pane's folder when local,
    /// else the pane's own folder, otherwise the home directory.
    pub fn ensure_terminal(&mut self, pane: PaneId) -> Result<(), FmError> {
        if self.terminals.contains_key(&pane) {
            return Ok(());
        }
        let own = self
            .layout
            .state(pane)
            .map(|p| p.location.clone())
            .ok_or_else(|| FmError::NotFound(format!("pane {pane}")))?;
        // Prefer the last active file pane's folder, then the pane's own, then home.
        let last_files = self
            .layout
            .focus
            .last_files
            .and_then(|id| self.layout.state(id))
            .filter(|p| p.kind.is_files())
            .and_then(|p| local_dir(&p.location));
        let cwd = last_files.or_else(|| local_dir(&own)).or_else(home_dir);
        let session = TermSession::spawn_with_scrollback(
            self.prefs.terminal_shell.to_term(),
            cwd.as_deref(),
            DEFAULT_ROWS,
            DEFAULT_COLS,
            self.prefs.terminal_scrollback_rows(),
        )?;
        self.terminals.insert(pane, session);
        Ok(())
    }

    /// Resize the pane's pty to the cells the UI can show.
    pub fn terminal_resize(&mut self, pane: PaneId, rows: u16, cols: u16) {
        if let Some(t) = self.terminals.get_mut(&pane) {
            let before = (t.snapshot().rows, t.snapshot().cols);
            t.resize(rows, cols);
            if (t.snapshot().rows, t.snapshot().cols) != before
                && self.terminal_selection.is_some_and(|sel| sel.pane == pane)
            {
                self.terminal_selection = None;
            }
        }
    }

    pub fn terminal_scroll(&mut self, pane: PaneId, lines: i32) {
        if let Some(t) = self.terminals.get_mut(&pane) {
            t.scroll(lines);
        }
    }

    pub fn terminal_select(&mut self, selection: Option<TermSelection>) {
        self.terminal_selection = selection;
    }

    /// Drop a pane's shell (pane closed or restarted); the child is killed.
    /// Whether the pane's terminal is running a command besides its shell.
    pub fn terminal_busy(&self, pane: PaneId) -> bool {
        self.terminals
            .get(&pane)
            .is_some_and(|t| t.has_foreground_job())
    }

    pub fn drop_terminal(&mut self, pane: PaneId) {
        self.terminals.remove(&pane);
        if self.terminal_selection.is_some_and(|s| s.pane == pane) {
            self.terminal_selection = None;
        }
    }

    /// Drain terminal output into the parsers. Returns `(changed, backlog)`.
    pub(crate) fn pump_terminals(&mut self) -> (bool, bool) {
        let mut total = Pump::default();
        for (&pane, t) in &mut self.terminals {
            let selected = self.terminal_selection.is_some_and(|sel| sel.pane == pane);
            let before = selected.then(|| t.snapshot());
            let p = t.pump();
            if p.changed
                && let Some(before) = before
            {
                let after = t.snapshot();
                if before.alternate_screen != after.alternate_screen
                    || before.scrollback_len > after.scrollback_len
                    || after.scrollback_len >= t.scrollback_capacity()
                {
                    self.terminal_selection = None;
                }
            }
            total.changed |= p.changed;
            total.backlog |= p.backlog;
        }
        (total.changed, total.backlog)
    }

    /// Close the panes whose shell has exited (`exit`, Ctrl-D). The last pane stays, showing
    /// the restart banner. Returns true when a pane was closed.
    pub(crate) fn close_exited_terminals(&mut self) -> bool {
        let exited: Vec<PaneId> = self
            .terminals
            .iter()
            .filter(|(_, t)| matches!(t.status(), Status::Exited(_)))
            .map(|(&pane, _)| pane)
            .collect();
        let mut closed = false;
        for pane in exited {
            if self.layout.panes().len() > 1 && self.layout.close(pane).is_ok() {
                self.drop_terminal(pane);
                closed = true;
            }
        }
        closed
    }

    /// Whether any terminal exists (the app keeps repainting to show output).
    pub fn has_terminals(&self) -> bool {
        !self.terminals.is_empty()
    }
}

fn terminal_focused(s: &Session) -> bool {
    s.focused_kind() == Some(PaneKind::Terminal)
}

fn target_pane(s: &Session, params: &Params) -> Result<PaneId, FmError> {
    match object(params)?.and_then(|m| m.get("pane")) {
        None | Some(Params::Null) => Ok(s.layout.focused()),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| FmError::InvalidParams("pane must be a number".to_string())),
    }
}

fn text_param<'a>(params: &'a Params, key: &str, cap: usize) -> Result<&'a str, FmError> {
    let text = object(params)?
        .and_then(|m| m.get(key))
        .and_then(Params::as_str)
        .ok_or_else(|| FmError::InvalidParams(format!("{key}: string required")))?;
    if text.len() > cap {
        return Err(FmError::InvalidParams(format!(
            "{key} larger than {cap} bytes"
        )));
    }
    Ok(text)
}

fn live_terminal(s: &mut Session, pane: PaneId) -> Result<&mut TermSession, FmError> {
    if s.pane_kind(pane) != Some(PaneKind::Terminal) {
        return Err(FmError::Unsupported("pane is not a terminal"));
    }
    s.terminals
        .get_mut(&pane)
        .ok_or(FmError::Unsupported("terminal is not running"))
}

/// `term.input` — send keystrokes to a terminal pane (internal; the UI's key translation).
pub fn input() -> CommandSpec {
    CommandSpec {
        id: "term.input",
        label: "Terminal Input",
        menu: &["Terminal", "Input"],
        shortcut: None,
        params: "{data: string, pane?: number}",
        enabled: terminal_focused,
        run: |s, p: &Params| {
            let pane = target_pane(s, p)?;
            let data = text_param(p, "data", MAX_KEYS_BYTES)?.to_string();
            live_terminal(s, pane)?.write(data.as_bytes())
        },
    }
}

/// `term.paste` — paste text, bracketed when the app asked for it.
pub fn paste() -> CommandSpec {
    CommandSpec {
        id: "term.paste",
        label: "Paste",
        menu: &["Terminal", "Paste"],
        shortcut: Some("Ctrl+Shift+V"),
        params: "{text: string, pane?: number}",
        enabled: terminal_focused,
        run: |s, p: &Params| {
            let pane = target_pane(s, p)?;
            let text = text_param(p, "text", MAX_PASTE_BYTES)?;
            paste_text(live_terminal(s, pane)?, text)
        },
    }
}

/// Drop every control character except tab/newline/CR: a single `replace` of the bracket
/// terminator can be rebuilt from surrounding text, letting the paste escape.
fn sanitize_paste(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect()
}

fn paste_text(term: &mut TermSession, text: &str) -> Result<(), FmError> {
    let clean = sanitize_paste(text);
    let mut bytes = Vec::with_capacity(clean.len() + 12);
    if term.snapshot().bracketed_paste {
        bytes.extend_from_slice(b"\x1b[200~");
        bytes.extend_from_slice(clean.as_bytes());
        bytes.extend_from_slice(b"\x1b[201~");
    } else {
        bytes.extend_from_slice(clean.replace("\r\n", "\r").replace('\n', "\r").as_bytes());
    }
    term.write(&bytes)
}

/// Most paths one drop may paste.
pub const MAX_DROP_PATHS: usize = 500;

/// `term.drop_paths` — files dropped on a terminal: paste their quoted local paths at the
/// cursor (never Enter), so the user decides what to run.
pub fn drop_paths() -> CommandSpec {
    CommandSpec {
        id: "term.drop_paths",
        label: "Paste Paths",
        menu: &["Terminal", "Paste Paths"],
        shortcut: None,
        params: "{paths: [uri or native path], pane?: number}",
        enabled: terminal_focused,
        run: |s, p: &Params| {
            let pane = target_pane(s, p)?;
            let list = object(p)?
                .and_then(|m| m.get("paths"))
                .and_then(Params::as_array)
                .ok_or_else(|| FmError::InvalidParams("paths: array required".to_string()))?;
            if list.len() > MAX_DROP_PATHS {
                return Err(FmError::InvalidParams(format!(
                    "more than {MAX_DROP_PATHS} paths"
                )));
            }
            let dialect = live_terminal(s, pane)?.dialect();
            let words: Vec<String> = list
                .iter()
                .filter_map(Params::as_str)
                .filter_map(|raw| VfsUri::parse(raw).ok())
                .filter_map(|uri| local_path(&uri))
                .filter_map(|path| fm_term::path_arg(dialect, &path).ok())
                .collect();
            if words.is_empty() {
                return Err(FmError::Unsupported(
                    "only local files can be dropped on a terminal",
                ));
            }
            let skipped = list.len() - words.len();
            let mut text = words.join(" ");
            text.push(' ');
            paste_text(live_terminal(s, pane)?, &text)?;
            if skipped > 0 {
                s.notice = Some(format!("{skipped} item(s) skipped: not local files"));
            }
            Ok(())
        },
    }
}

/// `term.copy` — put the selected text on the clipboard hand-off (`Session::clipboard_out`).
pub fn copy() -> CommandSpec {
    CommandSpec {
        id: "term.copy",
        label: "Copy",
        menu: &["Terminal", "Copy"],
        shortcut: Some("Ctrl+Shift+C"),
        params: "none",
        enabled: |s| terminal_focused(s) && s.terminal_selection.is_some(),
        run: |s, _p: &Params| {
            let Some(sel) = s
                .terminal_selection
                .filter(|x| x.pane == s.layout.focused())
            else {
                return Ok(()); // nothing selected: the menu entry is greyed out
            };
            let terminal = s
                .terminals
                .get_mut(&sel.pane)
                .ok_or(FmError::Unsupported("terminal is not running"))?;
            let text = terminal.text_between(sel.anchor, sel.head);
            if !text.is_empty() {
                s.clipboard_out = Some(text);
            }
            Ok(())
        },
    }
}

/// `term.cd_here` — `cd` the shell into the other pane's local directory.
pub fn cd_here() -> CommandSpec {
    CommandSpec {
        id: "term.cd_here",
        label: "Go to Other Pane's Folder",
        menu: &["Terminal", "cd to Other Pane"],
        shortcut: None,
        params: "none",
        enabled: |s| terminal_focused(s) && other_files_dir(s).is_some(),
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            if !terminal_focused(s) {
                return Ok(());
            }
            let dir = other_files_dir(s)
                .ok_or(FmError::Unsupported("no local folder in another pane"))?;
            let term = live_terminal(s, pane)?;
            let bytes = fm_term::cd_command(term.dialect(), &dir)?;
            term.write(&bytes)
        },
    }
}

fn other_files_dir(s: &Session) -> Option<String> {
    let uri = other_pane_dir(s)?;
    uri.local_path_string()
}

/// `term.clear_scrollback`
pub fn clear_scrollback() -> CommandSpec {
    CommandSpec {
        id: "term.clear_scrollback",
        label: "Clear Scrollback",
        menu: &["Terminal", "Clear Scrollback"],
        shortcut: None,
        params: "none",
        enabled: terminal_focused,
        run: |s, _p: &Params| {
            let pane = s.layout.focused();
            if !terminal_focused(s) {
                return Ok(());
            }
            live_terminal(s, pane)?.clear_scrollback();
            s.terminal_selection = None;
            Ok(())
        },
    }
}

/// `term.restart` — replace the shell (typically after it exited).
pub fn restart() -> CommandSpec {
    CommandSpec {
        id: "term.restart",
        label: "Restart Shell",
        menu: &["Terminal", "Restart Shell"],
        shortcut: None,
        params: "{pane?: number}",
        enabled: terminal_focused,
        run: |s, p: &Params| {
            let pane = target_pane(s, p)?;
            if s.pane_kind(pane) != Some(PaneKind::Terminal) {
                if matches!(p, Params::Null) {
                    return Ok(()); // implicit pane, not a terminal: greyed out
                }
                return Err(FmError::Unsupported("pane is not a terminal"));
            }
            s.drop_terminal(pane);
            s.ensure_terminal(pane)
        },
    }
}

/// Whether the pane's shell has exited (the UI shows the restart banner).
pub fn exit_code(s: &Session, pane: PaneId) -> Option<i32> {
    match s.terminal(pane)?.status() {
        Status::Exited(code) => Some(code),
        Status::Running => None,
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        input(),
        paste(),
        drop_paths(),
        copy(),
        cd_here(),
        clear_scrollback(),
        restart(),
    ]
}

#[cfg(test)]
mod paste_tests {
    use super::*;

    #[test]
    fn scrollback_preference_changes_new_and_restarted_sessions_only() {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").kind = PaneKind::Terminal;
        s.prefs.terminal_scrollback_rows = 5_000;
        s.ensure_terminal(pane).expect("start");
        assert_eq!(
            s.terminal(pane).expect("terminal").scrollback_capacity(),
            5_000
        );
        s.dispatch(
            "prefs.set",
            &serde_json::json!({
                "key": "terminal_scrollback_rows", "value": 1_000_000
            }),
        )
        .expect("set");
        s.ensure_terminal(pane).expect("keep existing");
        assert_eq!(
            s.terminal(pane).expect("terminal").scrollback_capacity(),
            5_000
        );
        s.dispatch("term.restart", &Params::Null).expect("restart");
        assert_eq!(
            s.terminal(pane).expect("terminal").scrollback_capacity(),
            1_000_000
        );
    }

    #[test]
    fn a_rebuilt_bracket_terminator_cannot_survive() {
        let out = sanitize_paste("\x1b[2\x1b[201~01~\nrm -rf ~\n");
        assert!(!out.contains('\x1b'));
        assert_eq!(sanitize_paste("a\tb\r\nc"), "a\tb\r\nc");
    }
}
