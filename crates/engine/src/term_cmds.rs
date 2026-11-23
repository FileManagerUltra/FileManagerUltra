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

    /// The current folder of the terminal shell `pane` runs (in any workspace tab), when it
    /// is an existing folder.
    pub(crate) fn terminal_dir(&self, pane: PaneId) -> Option<String> {
        self.terminals
            .get(&pane)
            .and_then(TermSession::cwd)
            .filter(|d| std::path::Path::new(d).is_dir())
    }

    /// Where a pane is working: a Files pane's local folder, or a terminal's current folder.
    pub fn pane_dir(&self, pane: PaneId) -> Option<String> {
        let state = self.layout.state(pane)?;
        if state.kind == PaneKind::Terminal {
            return self.terminal_dir(pane);
        }
        state
            .kind
            .is_files()
            .then(|| local_dir(&state.location))
            .flatten()
    }

    /// Start the pane's shell if it has none, in the first local folder of: the active pane
    /// (a pane being switched to Terminal counts as the Files pane it was), the pane active
    /// before it (a split's source terminal), the last Files folder in any workspace, home.
    /// A terminal pane's stored location is stale and never used; a terminal restored from a
    /// saved workspace starts in the folder it was saved in instead (`PaneState::term_dir`).
    pub fn ensure_terminal(&mut self, pane: PaneId) -> Result<(), FmError> {
        if self.terminals.contains_key(&pane) {
            return Ok(());
        }
        let Some(state) = self.layout.state_mut(pane) else {
            return Err(FmError::NotFound(format!("pane {pane}")));
        };
        // A terminal restored from a saved workspace starts where it was saved (once).
        if let Some(dir) = state
            .term_dir
            .take()
            .filter(|_| state.kind == PaneKind::Terminal)
            .and_then(|uri| local_dir(&uri))
        {
            return self.ensure_terminal_in(pane, Some(dir));
        }
        let (focused, previous) = (self.layout.focus.focused, self.layout.focus.previous);
        let candidate = |s: &Self, id: PaneId| {
            let switching = id == pane && s.pane_kind(id).is_some_and(PaneKind::is_files);
            (id != pane || switching).then(|| s.pane_dir(id)).flatten()
        };
        let cwd = candidate(self, focused)
            .or_else(|| previous.and_then(|id| candidate(self, id)))
            .or_else(|| self.last_files_dir.as_ref().and_then(local_dir))
            .or_else(home_dir);
        self.ensure_terminal_in(pane, cwd)
    }

    /// Start the pane's shell in `cwd` (the shell's default folder when `None`).
    fn ensure_terminal_in(&mut self, pane: PaneId, cwd: Option<String>) -> Result<(), FmError> {
        if self.terminals.contains_key(&pane) {
            return Ok(());
        }
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

    /// Remember the focused Files pane's local folder for new terminals in any workspace.
    pub(crate) fn note_files_dir(&mut self) {
        let focused = self.layout.focused();
        if let Some(state) = self.layout.state(focused).filter(|st| st.kind.is_files())
            && state.location.local_path().is_some()
            && self.last_files_dir.as_ref() != Some(&state.location)
        {
            self.last_files_dir = Some(state.location.clone());
        }
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
            // Only the focused pane may write the host clipboard; others are dropped.
            if let Some(text) = t.take_clipboard()
                && pane == self.layout.focused()
            {
                self.clipboard_out = Some(text);
            }
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
            term.write(&bytes)?;
            term.note_cwd(&dir);
            Ok(())
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
            // The new shell starts where the old one was.
            let cwd = s.pane_dir(pane);
            s.drop_terminal(pane);
            match cwd {
                Some(dir) => s.ensure_terminal_in(pane, Some(dir)),
                None => s.ensure_terminal(pane),
            }
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

#[cfg(test)]
mod start_folder_tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn dir_uri(dir: &std::path::Path) -> VfsUri {
        VfsUri::parse(dir.to_str().expect("utf8")).expect("uri")
    }

    fn canon(dir: &std::path::Path) -> String {
        dir.canonicalize()
            .expect("canonical")
            .to_str()
            .expect("utf8")
            .to_string()
    }

    fn started_in(s: &Session, pane: PaneId) -> Option<String> {
        s.terminal(pane).and_then(TermSession::cwd).map(|d| {
            std::path::Path::new(&d)
                .canonicalize()
                .map_or(d, |p| p.to_string_lossy().into_owned())
        })
    }

    /// One pane at `dir`, switched to Terminal: the shell starts there.
    fn terminal_at(dir: &std::path::Path) -> Option<(Session, PaneId)> {
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").location = dir_uri(dir);
        s.dispatch("pane.kind.set", &serde_json::json!({"kind": "terminal"}))
            .ok()?;
        Some((s, pane))
    }

    #[test]
    fn a_files_pane_switched_to_terminal_starts_in_its_folder() {
        let a = tempfile::tempdir().expect("tmp");
        let Some((s, pane)) = terminal_at(a.path()) else {
            return;
        };
        assert_eq!(started_in(&s, pane), Some(canon(a.path())));
    }

    #[test]
    fn a_split_terminal_starts_where_its_source_terminal_is_not_its_stale_location() {
        let (a, b) = (
            tempfile::tempdir().expect("a"),
            tempfile::tempdir().expect("b"),
        );
        let Some((mut s, source)) = terminal_at(a.path()) else {
            return;
        };
        let expected = if cfg!(target_os = "linux") {
            // The shell moves on; the pane's stored location stays at `a`.
            let path = canon(b.path());
            let term = s.terminals.get_mut(&source).expect("terminal");
            term.write(format!("cd '{path}'\r").as_bytes()).expect("cd");
            let deadline = Instant::now() + Duration::from_secs(10);
            while started_in(&s, source).as_deref() != Some(path.as_str()) {
                assert!(Instant::now() < deadline, "shell never moved");
                s.poll();
                std::thread::sleep(Duration::from_millis(10));
            }
            path
        } else {
            canon(a.path())
        };
        s.dispatch("pane.split.horizontal", &Params::Null)
            .expect("split");
        let new = s.layout.focused();
        assert_ne!(new, source);
        s.ensure_terminal(new).expect("start");
        assert_eq!(started_in(&s, new), Some(expected));
    }

    /// Send the pane's shell to `dir` and wait until the app sees it there: Linux reads the
    /// real folder, Windows shells report it from their prompt; elsewhere the app is told.
    fn move_shell(s: &mut Session, pane: PaneId, dir: &str) {
        let term = s.terminals.get_mut(&pane).expect("terminal");
        if cfg!(windows) {
            term.write(format!("Set-Location -LiteralPath '{dir}'\r").as_bytes())
                .expect("cd");
        } else if cfg!(target_os = "linux") {
            term.write(format!("cd '{dir}'\r").as_bytes()).expect("cd");
        } else {
            term.note_cwd(dir);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while started_in(s, pane).as_deref() != Some(dir) {
            assert!(
                Instant::now() < deadline,
                "shell never reached {dir}: {:?}",
                started_in(s, pane)
            );
            s.poll();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_saved_workspace_restores_each_terminal_in_its_folder() {
        let (a, b) = (
            tempfile::tempdir().expect("a"),
            tempfile::tempdir().expect("b"),
        );
        let Some((mut s, pane)) = terminal_at(a.path()) else {
            return;
        };
        let (at_a, at_b) = (canon(a.path()), canon(b.path()));
        move_shell(&mut s, pane, &at_b);
        s.dispatch("workspace.save", &serde_json::json!({"name": "Shells"}))
            .expect("save");
        s.dispatch("workspace.startup", &Params::Null)
            .expect("startup");
        let text = s.workspace_tick().expect("saved file");
        // Moving the shell afterwards is not an unsaved change of the workspace.
        move_shell(&mut s, pane, &at_a);
        assert!(s.workspace_unsaved().is_empty(), "a cd marked it unsaved");

        let mut t = Session::new().expect("session");
        assert!(t.workspace_startup(&text).is_none());
        let restored = t
            .layout
            .panes()
            .into_iter()
            .find(|p| t.pane_kind(*p) == Some(PaneKind::Terminal))
            .expect("terminal pane");
        t.ensure_terminal(restored).expect("start");
        assert_eq!(
            started_in(&t, restored),
            Some(at_b),
            "starts where it was saved"
        );
        // Once: a restart afterwards follows the shell, not the saved folder.
        assert!(
            t.layout
                .state(restored)
                .is_some_and(|st| st.term_dir.is_none())
        );
    }

    #[test]
    fn with_no_pane_to_follow_the_last_files_folder_of_any_workspace_is_used() {
        let (stale, last) = (
            tempfile::tempdir().expect("a"),
            tempfile::tempdir().expect("b"),
        );
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        {
            let st = s.layout.state_mut(pane).expect("pane");
            st.kind = PaneKind::Terminal;
            st.location = dir_uri(stale.path());
        }
        s.layout.focus.previous = None;
        s.last_files_dir = Some(dir_uri(last.path()));
        if s.ensure_terminal(pane).is_err() {
            return;
        }
        assert_eq!(started_in(&s, pane), Some(canon(last.path())));
    }

    #[test]
    fn the_last_files_folder_follows_the_focused_files_pane() {
        let a = tempfile::tempdir().expect("a");
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        s.layout.state_mut(pane).expect("pane").location = dir_uri(a.path());
        s.poll();
        assert_eq!(s.last_files_dir, Some(dir_uri(a.path())));
    }

    #[test]
    fn restart_keeps_the_folder() {
        let a = tempfile::tempdir().expect("a");
        let Some((mut s, pane)) = terminal_at(a.path()) else {
            return;
        };
        let before = started_in(&s, pane);
        s.dispatch("term.restart", &Params::Null).expect("restart");
        assert_eq!(started_in(&s, pane), before);
    }
}
