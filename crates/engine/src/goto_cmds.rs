//! Editable location bar: `nav.goto` starts editing the focused pane's path, completions
//! come from the cached directory listings (never a blocking read), visited locations are
//! remembered for the session.

use fm_layout::PaneId;
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::file_cmds::object;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Visited locations kept for suggestions (most recent first).
pub const MAX_HISTORY: usize = 100;
const MAX_DIR_SUGGESTIONS: usize = 30;
const MAX_HISTORY_SUGGESTIONS: usize = 8;
/// Typed paths longer than this are not completed (hostile input).
const MAX_EDIT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    /// A sub-directory of the directory being typed in: accepting it keeps editing.
    Dir,
    /// A previously visited location: accepting it navigates.
    History,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
}

/// The path being edited in a pane's location bar.
#[derive(Clone, Debug)]
pub struct PathEdit {
    pub pane: PaneId,
    pub text: String,
    pub suggestions: Vec<Suggestion>,
    /// Highlighted suggestion (arrow keys); `None` means "what I typed".
    pub selected: Option<usize>,
    /// The UI grabs keyboard focus on the first frame only.
    pub focus_pending: bool,
    /// Programmatic text change: the UI moves the caret to the end once.
    pub caret_to_end: bool,
    computed_for: Option<String>,
    complete: bool,
}

impl PathEdit {
    fn new(pane: PaneId, text: String) -> Self {
        Self {
            pane,
            text,
            suggestions: Vec::new(),
            selected: None,
            focus_pending: true,
            caret_to_end: true,
            computed_for: None,
            complete: false,
        }
    }

    /// Replace the text from code (accepting a suggestion).
    pub fn set_text(&mut self, text: String) {
        self.text = text;
        self.selected = None;
        self.caret_to_end = true;
    }

    /// Move the highlight by `delta` (wrapping through "nothing selected").
    pub fn step(&mut self, delta: isize) {
        let n = self.suggestions.len() as isize;
        if n == 0 {
            self.selected = None;
            return;
        }
        let cur = self.selected.map_or(-1, |i| i as isize);
        let next = cur + delta;
        self.selected = if next < 0 || next >= n {
            None
        } else {
            usize::try_from(next).ok()
        };
    }
}

pub(crate) fn home_dir() -> Option<String> {
    directories::BaseDirs::new().map(|b| b.home_dir().to_string_lossy().into_owned())
}

fn is_sep(c: char) -> bool {
    c == '/' || c == '\\'
}

/// A path that stands on its own: absolute, a drive path, a UNC path or a `scheme://` uri.
fn is_anchored(text: &str) -> bool {
    let b = text.as_bytes();
    text.starts_with(is_sep)
        || text.contains("://")
        || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':')
}

/// Resolve what the user typed into a uri: `~` is the home directory, relative paths are
/// relative to `base`, `.` and `..` segments are applied, local and remote forms both work.
/// Windows-only forms: a bare drive (`D:`) is its root, `This PC` is the drive list.
fn windows_shortcut(text: &str, windows: bool) -> Option<VfsUri> {
    if !windows {
        return None;
    }
    if text.eq_ignore_ascii_case("this pc") {
        return Some(VfsUri::root_file());
    }
    match text.as_bytes() {
        [d, b':'] if d.is_ascii_alphabetic() => VfsUri::parse(&format!("{text}\\")).ok(),
        _ => None,
    }
}

pub fn resolve(base: &VfsUri, text: &str) -> Result<VfsUri, FmError> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_EDIT_BYTES {
        return Err(FmError::invalid_uri("empty or oversized path"));
    }
    let expanded;
    let text = if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        let home = home_dir().ok_or_else(|| FmError::NotFound("home directory".to_string()))?;
        expanded = format!("{home}{}", &text[1..]);
        expanded.as_str()
    } else {
        text
    };
    if let Some(uri) = windows_shortcut(text, cfg!(windows)) {
        return Ok(uri);
    }
    // Windows already reaches `\\server\share` natively; elsewhere (and when typed into a
    // network pane) it opens the in-app SMB connection.
    if (!cfg!(windows) || matches!(base.scheme(), Scheme::Smb | Scheme::Network))
        && let Some(uri) = unc_to_smb(text)?
    {
        return Ok(uri);
    }
    if is_anchored(text) {
        return VfsUri::parse(text);
    }
    let mut cur = base.clone();
    for seg in text.split(is_sep).filter(|s| !s.is_empty() && *s != ".") {
        cur = if seg == ".." {
            cur.parent().unwrap_or(cur)
        } else {
            cur.join(seg)?
        };
    }
    Ok(cur)
}

/// `\\host\share\dir` as `smb://host/share/dir`; `None` when `text` is not a UNC path.
fn unc_to_smb(text: &str) -> Result<Option<VfsUri>, FmError> {
    let Some(rest) = text.strip_prefix("\\\\") else {
        return Ok(None);
    };
    if rest.starts_with(['?', '.']) {
        return Ok(None);
    }
    let mut segments = rest.split('\\').filter(|s| !s.is_empty());
    let host = segments
        .next()
        .ok_or_else(|| FmError::invalid_uri("missing server name"))?;
    let mut uri = VfsUri::parse(&format!("smb://{host}/"))?;
    for seg in segments {
        uri = uri.join(seg)?;
    }
    Ok(Some(uri))
}

/// Split typed text into the directory part (including the trailing separator) and the
/// name prefix being completed.
fn split_typed(text: &str) -> (&str, &str) {
    match text.rfind(is_sep) {
        Some(i) => text.split_at(i + 1),
        None => ("", text),
    }
}

fn display_of(uri: &VfsUri) -> String {
    if uri.scheme() == Scheme::File {
        crate::item_cmds::display_path(uri)
    } else {
        uri.as_str()
    }
}

impl Session {
    /// Remember a visited location (most recent first, no duplicates, capped).
    pub fn note_visit(&mut self, uri: &VfsUri) {
        let shown = display_of(uri);
        self.path_history.retain(|p| p != &shown);
        self.path_history.insert(0, shown);
        self.path_history.truncate(MAX_HISTORY);
    }

    /// Recompute the suggestions for the open path edit when its text changed or the
    /// directory it completes in has finished loading. Never blocks: a directory that is not
    /// cached yet is queued for reading and the list fills in on a later frame.
    pub fn refresh_suggestions(&mut self) {
        let Some(edit) = self.path_edit.as_ref() else {
            return;
        };
        if edit.computed_for.as_deref() == Some(edit.text.as_str()) && edit.complete {
            return;
        }
        let text = edit.text.clone();
        let pane = edit.pane;
        let base = self
            .layout
            .state(pane)
            .map(|p| p.location.clone())
            .unwrap_or_else(VfsUri::root_file);
        let (list, complete) = self.compute_suggestions(&base, &text);
        if let Some(edit) = self.path_edit.as_mut() {
            edit.suggestions = list;
            edit.computed_for = Some(text);
            edit.complete = complete;
            if edit.selected.is_some_and(|i| i >= edit.suggestions.len()) {
                edit.selected = None;
            }
        }
    }

    fn compute_suggestions(&mut self, base: &VfsUri, text: &str) -> (Vec<Suggestion>, bool) {
        let mut out: Vec<Suggestion> = Vec::new();
        let mut complete = true;
        if text.len() <= MAX_EDIT_BYTES {
            let (dir_text, prefix) = split_typed(text);
            let dir = if dir_text.is_empty() {
                Ok(base.clone())
            } else {
                resolve(base, dir_text)
            };
            if let Ok(dir) = dir {
                match self.dirs.state(&dir).entries() {
                    Some(entries) => {
                        let lower = prefix.to_lowercase();
                        let want_hidden = self.show_hidden || prefix.starts_with('.');
                        let mut names: Vec<&str> = entries
                            .iter()
                            .filter(|e| e.is_dir_like())
                            .filter(|e| want_hidden || !crate::session::is_hidden(e))
                            .map(|e| e.name.as_str())
                            .filter(|n| n.to_lowercase().starts_with(&lower))
                            .collect();
                        names.sort_by_key(|n| n.to_lowercase());
                        names.truncate(MAX_DIR_SUGGESTIONS);
                        out.extend(names.into_iter().map(|n| Suggestion {
                            text: format!("{dir_text}{n}/"),
                            kind: SuggestionKind::Dir,
                        }));
                    }
                    None => {
                        complete = false;
                        // Queue the read; a failure just means no completions.
                        if self.backends.require(&dir).is_ok()
                            && self.ensure_read(dir.clone()).is_err()
                        {
                            complete = true;
                        }
                        if matches!(self.dirs.state(&dir), crate::DirState::Failed { .. }) {
                            complete = true;
                        }
                    }
                }
            }
        }
        let lower = text.to_lowercase();
        let mut taken = 0;
        for shown in &self.path_history {
            if taken >= MAX_HISTORY_SUGGESTIONS {
                break;
            }
            let hay = shown.to_lowercase();
            let matches = lower.is_empty() || hay.contains(&lower);
            let dup = out
                .iter()
                .any(|s| s.text.trim_end_matches(is_sep) == shown.as_str());
            if matches && !dup && shown.as_str() != text.trim_end_matches(is_sep) {
                out.push(Suggestion {
                    text: shown.clone(),
                    kind: SuggestionKind::History,
                });
                taken += 1;
            }
        }
        (out, complete)
    }
}

/// `nav.goto` — edit the focused pane's path with completion and history.
pub fn goto() -> CommandSpec {
    CommandSpec {
        id: "nav.goto",
        label: "Go to Path…",
        menu: &["Go", "Go to Path…"],
        shortcut: Some("Ctrl+L"),
        params: "{\"text\"?: string} seeds the editor instead of the current path",
        enabled: |s| s.focused_is_files(),
        run: |s, p: &Params| {
            let pane = s.layout.focused();
            let seed = match p {
                Params::Null => None,
                Params::Object(m) => match m.get("text") {
                    None | Some(Params::Null) => None,
                    Some(Params::String(t)) if t.len() <= 4096 => Some(t.clone()),
                    Some(_) => {
                        return Err(FmError::InvalidParams("text must be a short string".into()));
                    }
                },
                _ => return Err(FmError::InvalidParams("params must be an object".into())),
            };
            if s.path_edit.as_ref().is_some_and(|e| e.pane == pane) {
                return Ok(());
            }
            if let Some(text) = seed {
                s.path_edit = Some(PathEdit::new(pane, text));
                return Ok(());
            }
            let loc = s
                .layout
                .state(pane)
                .map(|p| display_of(&p.location))
                .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
            let text = if loc.ends_with('/') {
                loc
            } else {
                format!("{loc}/")
            };
            s.path_edit = Some(PathEdit::new(pane, text));
            Ok(())
        },
    }
}

/// `nav.goto.commit` — navigate the focused pane to a typed path and stop editing. On a bad
/// path the edit stays open so it can be corrected.
pub fn commit() -> CommandSpec {
    CommandSpec {
        id: "nav.goto.commit",
        label: "Go to Typed Path",
        menu: &["Go", "Go to Typed Path"],
        shortcut: None,
        params: "path: string",
        enabled: |s| s.path_edit.is_some(),
        run: |s, params: &Params| {
            let raw = object(params)?
                .and_then(|m| m.get("path"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| FmError::InvalidParams("path: string required".to_string()))?
                .to_string();
            let base = s
                .layout
                .state(s.layout.focused())
                .map(|p| p.location.clone())
                .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
            let target = resolve(&base, &raw)?;
            // A typed `sftp://` or `smb://` address with no connection yet asks in the sign-in popup.
            if matches!(target.scheme(), Scheme::Smb | Scheme::Sftp)
                && s.backends.require(&target).is_err()
            {
                let focused = s.layout.focused();
                if let Some(state) = s.layout.state_mut(focused)
                    && state.kind.is_files()
                {
                    state.navigate(target.clone());
                }
                if !s.needs_login(&target) {
                    s.needs_sftp_login(&target);
                }
                s.path_edit = None;
                return Ok(());
            }
            s.dispatch("nav.open", &Params::String(display_of(&target)))?;
            s.path_edit = None;
            Ok(())
        },
    }
}

/// `nav.goto.cancel` — stop editing the path.
pub fn cancel() -> CommandSpec {
    CommandSpec {
        id: "nav.goto.cancel",
        label: "Cancel Path Edit",
        menu: &["Go", "Cancel Path Edit"],
        shortcut: None,
        params: "none",
        enabled: |s| s.path_edit.is_some(),
        run: |s, _p: &Params| {
            s.path_edit = None;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![goto(), commit(), cancel()]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use fm_testkit::MemFs;
    use serde_json::json;

    use super::*;

    fn session() -> (Session, VfsUri) {
        let mem = MemFs::new();
        for d in [
            "/home/alice/Documents",
            "/home/alice/Downloads",
            "/home/alice/.cache",
        ] {
            mem.mkdirs_str(d).expect("mkdirs");
        }
        mem.put_file(&VfsUri::parse("/home/alice/notes.txt").expect("uri"), b"x")
            .expect("file");
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(mem));
        let home = VfsUri::parse("/home/alice").expect("uri");
        if let Some(p) = s.layout.state_mut(s.layout.focused()) {
            p.location = home.clone();
        }
        (s, home)
    }

    fn settle(s: &mut Session) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            s.poll();
            s.refresh_suggestions();
            if s.path_edit.as_ref().is_some_and(|e| e.complete) {
                return;
            }
            assert!(Instant::now() < deadline, "suggestions never completed");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn texts(s: &Session) -> Vec<String> {
        s.path_edit
            .as_ref()
            .map(|e| e.suggestions.iter().map(|x| x.text.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn windows_drive_and_this_pc_shortcuts() {
        let d = windows_shortcut("d:", true).expect("drive");
        assert_eq!(d.path_str(), "/d:");
        assert_eq!(windows_shortcut("This PC", true), Some(VfsUri::root_file()));
        assert_eq!(windows_shortcut("d:", false), None);
        assert_eq!(windows_shortcut("dd:", true), None);
        assert_eq!(windows_shortcut("1:", true), None);
    }

    #[test]
    fn resolve_handles_relative_dots_home_and_absolute() {
        let base = VfsUri::parse("/home/alice/docs").expect("uri");
        let p = |t: &str| resolve(&base, t).expect(t).path_str().to_string();
        assert_eq!(p("/etc"), "/etc");
        assert_eq!(p("sub/x"), "/home/alice/docs/sub/x");
        assert_eq!(p("../bob"), "/home/alice/bob");
        assert_eq!(p("./a/../b"), "/home/alice/docs/b");
        assert_eq!(p(".."), "/home/alice");
        assert!(resolve(&base, "").is_err());
        assert!(resolve(&base, &"a/".repeat(5000)).is_err());
        assert!(resolve(&base, "bad\u{7}name").is_err());
    }

    #[test]
    #[cfg(not(windows))]
    fn unc_paths_open_the_in_app_smb_connection() {
        let base = VfsUri::parse("/home/alice").expect("uri");
        let host = resolve(&base, r"\\192.168.0.50").expect("host");
        assert_eq!(host.as_str(), "smb://192.168.0.50/");
        let dir = resolve(&base, r"\\nas\media\Movies\2024").expect("dir");
        assert_eq!(dir.as_str(), "smb://nas/media/Movies/2024");
        assert!(resolve(&base, r"\\").is_err());
        assert!(resolve(&base, r"\\nas\a\..\x").is_err());
    }

    #[test]
    fn goto_opens_an_edit_on_the_focused_pane_with_the_current_path() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        let edit = s.path_edit.as_ref().expect("edit");
        assert_eq!(edit.text, "/home/alice/");
        s.dispatch("nav.goto.cancel", &Params::Null)
            .expect("cancel");
        assert!(s.path_edit.is_none());
    }

    #[test]
    fn goto_with_text_seeds_the_editor_and_rejects_bad_params() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &serde_json::json!({ "text": "Doc" }))
            .expect("goto");
        assert_eq!(s.path_edit.as_ref().map(|e| e.text.as_str()), Some("Doc"));
        s.path_edit = None;
        assert!(
            s.dispatch("nav.goto", &serde_json::json!({ "text": 5 }))
                .is_err()
        );
        assert!(s.dispatch("nav.goto", &serde_json::json!("x")).is_err());
    }

    #[test]
    fn completes_directories_only_and_hides_dot_dirs_until_asked() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        settle(&mut s);
        assert_eq!(
            texts(&s),
            vec!["/home/alice/Documents/", "/home/alice/Downloads/"],
            "no files, no dot dirs"
        );
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text("/home/alice/do".to_string());
        }
        settle(&mut s);
        assert_eq!(texts(&s).len(), 2, "case-insensitive prefix");
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text("/home/alice/.".to_string());
        }
        settle(&mut s);
        assert_eq!(texts(&s), vec!["/home/alice/.cache/"]);
    }

    #[test]
    fn relative_text_completes_against_the_pane_location() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text("Docu".to_string());
        }
        settle(&mut s);
        assert_eq!(texts(&s), vec!["Documents/"]);
    }

    #[test]
    fn history_is_recent_first_deduplicated_and_filtered() {
        let (mut s, home) = session();
        let docs = home.join("Documents").expect("uri");
        s.note_visit(&docs);
        s.note_visit(&home);
        s.note_visit(&docs);
        assert_eq!(s.path_history, vec!["/home/alice/Documents", "/home/alice"]);
        s.path_edit = Some(PathEdit::new(
            s.layout.focused(),
            "/home/alice/".to_string(),
        ));
        settle(&mut s);
        let history: Vec<_> = s
            .path_edit
            .as_ref()
            .map(|e| {
                e.suggestions
                    .iter()
                    .filter(|x| x.kind == SuggestionKind::History)
                    .map(|x| x.text.clone())
                    .collect()
            })
            .unwrap_or_default();
        assert!(
            history.is_empty(),
            "already offered as a directory / is the current text"
        );
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text("documents".to_string());
        }
        settle(&mut s);
        assert!(
            s.path_edit
                .as_ref()
                .is_some_and(|e| e.suggestions.iter().any(
                    |x| x.kind == SuggestionKind::History && x.text == "/home/alice/Documents"
                )),
            "history matches anywhere in the path"
        );
        for i in 0..(MAX_HISTORY + 20) {
            s.note_visit(&VfsUri::parse(&format!("/tmp/{i}")).expect("uri"));
        }
        assert_eq!(s.path_history.len(), MAX_HISTORY);
    }

    #[test]
    fn selection_steps_through_suggestions_and_back_to_typed_text() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        settle(&mut s);
        let e = s.path_edit.as_mut().expect("edit");
        e.step(1);
        assert_eq!(e.selected, Some(0));
        e.step(1);
        assert_eq!(e.selected, Some(1));
        e.step(1);
        assert_eq!(e.selected, None, "past the end returns to the typed text");
        e.step(-1);
        assert_eq!(e.selected, None);
    }

    #[test]
    fn commit_navigates_and_records_history_and_bad_paths_keep_the_edit() {
        let (mut s, home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        s.dispatch("nav.goto.commit", &json!({"path": "Documents"}))
            .expect("commit");
        assert!(s.path_edit.is_none());
        let docs = home.join("Documents").expect("uri");
        assert_eq!(
            s.layout.state(s.layout.focused()).expect("pane").location,
            docs
        );
        assert_eq!(
            s.path_history.first().map(String::as_str),
            Some("/home/alice/Documents")
        );

        s.dispatch("nav.goto", &Params::Null).expect("goto");
        assert!(
            s.dispatch("nav.goto.commit", &json!({"path": "bad\u{7}"}))
                .is_err()
        );
        assert!(s.path_edit.is_some(), "edit stays open to be corrected");
    }

    #[test]
    fn commit_rejects_malformed_params() {
        let (mut s, _home) = session();
        for bad in [
            json!(null),
            json!({}),
            json!({"path": 3}),
            json!("x"),
            json!([1]),
        ] {
            assert!((commit().run)(&mut s, &bad).is_err(), "{bad}");
        }
    }
}
