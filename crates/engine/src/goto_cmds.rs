//! Editable location bar: `nav.goto` starts editing the focused pane's path, completions
//! come from the cached directory listings (never a blocking read), visited locations are
//! remembered for the session.

use fm_layout::PaneId;
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::file_cmds::object;
use crate::item_cmds::display_path;
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
        || matches!(b, [drive, b':', ..] if drive.is_ascii_alphabetic())
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

/// [`resolve_with`] for this OS, with no in-app SMB connection (tests and pure callers).
pub fn resolve(base: &VfsUri, text: &str) -> Result<VfsUri, FmError> {
    resolve_with(base, text, cfg!(windows), &|_| false)
}

/// Resolve typed text on an OS (`windows`). `smb_connected(host)` says whether the app has
/// its own SMB connection to `host`: on Windows a `\\host\share` path then means that
/// connection (it is how such a location is shown), otherwise Windows' own access.
pub fn resolve_with(
    base: &VfsUri,
    text: &str,
    windows: bool,
    smb_connected: &dyn Fn(&str) -> bool,
) -> Result<VfsUri, FmError> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_EDIT_BYTES {
        return Err(FmError::invalid_uri("empty or oversized path"));
    }
    let expanded;
    let text = if text == "~" || text.starts_with("~/") || text.starts_with("~\\") {
        let home = home_dir().ok_or_else(|| FmError::NotFound("home directory".to_string()))?;
        expanded = format!("{home}{}", text.strip_prefix('~').unwrap_or(text));
        expanded.as_str()
    } else {
        text
    };
    if let Some(uri) = windows_shortcut(text, windows) {
        return Ok(uri);
    }
    // Elsewhere `\\server\share` opens the in-app SMB connection. Windows reaches it
    // natively, unless the app is connected to that server or the pane is a network one.
    let in_app = |host: &str| {
        !windows || matches!(base.scheme(), Scheme::Smb | Scheme::Network) || smb_connected(host)
    };
    if unc_host(text).is_none_or(in_app)
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

/// The server of a UNC path (`\\host\share` → `host`); `None` for other text.
fn unc_host(text: &str) -> Option<&str> {
    let rest = text.strip_prefix("\\\\")?;
    if rest.starts_with(['?', '.']) {
        return None;
    }
    rest.split('\\').find(|s| !s.is_empty())
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

/// The separator the editor puts after a folder: the one the folder's display form uses, so
/// the text stays one style (`C:\Users\Documents\`, `\\nas\media\` on Windows), `/`
/// for uris and POSIX paths.
fn sep_of(uri: &VfsUri) -> char {
    let shown = display_path(uri);
    if !shown.contains("://") && shown.contains('\\') {
        '\\'
    } else {
        '/'
    }
}

impl Session {
    /// Remember a visited location (most recent first, no duplicates, capped).
    pub fn note_visit(&mut self, uri: &VfsUri) {
        self.path_history.retain(|p| p != uri);
        self.path_history.insert(0, uri.clone());
        self.path_history.truncate(MAX_HISTORY);
    }

    /// Whether the app has its own SMB connection to `host` (no user, default port).
    fn smb_connected(&self, host: &str) -> bool {
        VfsUri::parse(&format!("smb://{host}/"))
            .is_ok_and(|uri| self.backends.get_for(&uri).is_some())
    }

    /// Turn typed text into a location, relative to `base`: every display form comes back to
    /// the location it is shown for (see [`resolve_with`]).
    pub fn resolve_typed(&self, base: &VfsUri, text: &str) -> Result<VfsUri, FmError> {
        resolve_with(base, text, cfg!(windows), &|host| self.smb_connected(host))
    }

    /// [`Self::resolve_typed`] for fields that take a whole location (the Find Files folder):
    /// relative text is an error instead of being joined to a folder.
    pub fn resolve_absolute(&self, text: &str) -> Result<VfsUri, FmError> {
        let text = text.trim();
        let absolute = text == "~"
            || text.starts_with("~/")
            || text.starts_with("~\\")
            || is_anchored(text)
            || windows_shortcut(text, cfg!(windows)).is_some();
        if !absolute {
            return Err(FmError::InvalidParams(format!(
                "“{text}” is not a full path; type it from the drive or root"
            )));
        }
        self.resolve_typed(&VfsUri::root_file(), text)
    }

    /// Like [`Self::resolve_typed`], but text that is exactly a recent location as shown
    /// returns that location, whatever is connected now.
    fn resolve_committed(&self, base: &VfsUri, text: &str) -> Result<VfsUri, FmError> {
        let trimmed = text.trim().trim_end_matches(is_sep);
        let recent = self.path_history.iter().find(|uri| {
            let shown = display_path(uri);
            !trimmed.is_empty() && shown.trim_end_matches(is_sep) == trimmed
        });
        match recent {
            Some(uri) => Ok(uri.clone()),
            None => self.resolve_typed(base, text),
        }
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
                self.resolve_typed(base, dir_text)
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
                        let sep = sep_of(&dir);
                        out.extend(names.into_iter().map(|n| Suggestion {
                            text: format!("{dir_text}{n}{sep}"),
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
        for uri in &self.path_history {
            if taken >= MAX_HISTORY_SUGGESTIONS {
                break;
            }
            let shown = display_path(uri);
            let hay = shown.to_lowercase();
            let matches = lower.is_empty() || hay.contains(&lower);
            let dup = out
                .iter()
                .any(|s| s.text.trim_end_matches(is_sep) == shown.as_str());
            if matches && !dup && shown.as_str() != text.trim_end_matches(is_sep) {
                out.push(Suggestion {
                    text: shown,
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
            let location = s
                .layout
                .state(pane)
                .map(|p| p.location.clone())
                .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?;
            let loc = display_path(&location);
            let text = if loc.ends_with(is_sep) {
                loc
            } else {
                format!("{loc}{}", sep_of(&location))
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
            let target = s.resolve_committed(&base, &raw)?;
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
            s.dispatch("nav.open", &Params::String(target.as_str()))?;
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

    /// A test path as the editor shows it on this OS: `/home/alice/` is
    /// `C:\home\alice\` on Windows (a bare `/home` would be a UNC server there).
    fn native(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_string()
        }
    }

    fn session() -> (Session, VfsUri) {
        let mem = MemFs::new();
        for d in [
            "/home/alice/Documents",
            "/home/alice/Downloads",
            "/home/alice/.cache",
        ] {
            mem.mkdirs_str(&native(d)).expect("mkdirs");
        }
        mem.put_file(
            &VfsUri::parse(&native("/home/alice/notes.txt")).expect("uri"),
            b"x",
        )
        .expect("file");
        let mut s = Session::new().expect("session");
        s.backends.register(Arc::new(mem));
        let home = VfsUri::parse(&native("/home/alice")).expect("uri");
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
    fn windows_unc_uses_the_app_connection_only_when_there_is_one() {
        let local = VfsUri::parse(r"C:\Users\alice").expect("uri");
        let none = |_: &str| false;
        let nas = |host: &str| host == "nas";
        let go = |base: &VfsUri, text: &str, connected: &dyn Fn(&str) -> bool| {
            resolve_with(base, text, true, connected)
                .expect(text)
                .as_str()
        };
        assert_eq!(
            go(&local, r"\\nas\media\Movies", &nas),
            "smb://nas/media/Movies"
        );
        assert_eq!(go(&local, r"\\nas", &nas), "smb://nas/");
        // Not connected: Windows' own access, as before (also for `..`, which it normalizes).
        assert_eq!(go(&local, r"\\nas\media", &none), "file:///nas/media");
        assert_eq!(go(&local, r"\\other\public", &nas), "file:///other/public");
        assert_eq!(go(&local, r"\\nas\a\..\x", &none), "file:///nas/x");
        // A network pane keeps meaning the app's connection.
        let smb = VfsUri::parse("smb://nas/media").expect("uri");
        assert_eq!(go(&smb, r"\\fileserver\x", &none), "smb://fileserver/x");
        // Windows shortcuts.
        assert_eq!(go(&local, "This PC", &none), "file:///");
        assert_eq!(go(&local, "D:", &none), "file:///D:");
    }

    #[test]
    fn every_display_form_resolves_back_to_its_location() {
        let base = VfsUri::parse("/").expect("uri");
        let all = |_: &str| true;
        for (windows, uris) in [
            (
                true,
                &[
                    "file:///C:/Users/alice",
                    "file:///C:",
                    "file:///",
                    "smb://nas/media/Movies",
                    "smb://192.168.0.50/",
                    "smb://alice@nas/media",
                    "smb://nas:1445/media",
                    "sftp://bob@host/srv/www",
                    "sftp://bob@host:2222/",
                ][..],
            ),
            (
                false,
                &[
                    "file:///home/alice",
                    "file:///",
                    "smb://nas/media/Movies",
                    "smb://alice@nas/media",
                    "sftp://bob@host/srv/www",
                ][..],
            ),
        ] {
            for raw in uris {
                let uri = VfsUri::parse(raw).expect(raw);
                let shown = if windows && uri.as_str() == "file:///" {
                    "This PC".to_string()
                } else {
                    fm_vfs::display_form(&uri, windows)
                };
                let back = resolve_with(&base, &shown, windows, &all).expect(&shown);
                assert_eq!(back, uri, "{shown} (windows: {windows})");
            }
        }
    }

    #[test]
    fn a_recent_location_commits_to_where_it_was_recorded() {
        let (mut s, _home) = session();
        let share = VfsUri::parse("smb://nas/media").expect("uri");
        s.note_visit(&share);
        let base = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        let shown = display_path(&share);
        assert_eq!(s.resolve_committed(&base, &shown).expect("recent"), share);
        // The same text with a trailing separator, as a suggestion leaves it.
        let with_sep = format!("{shown}{}", sep_of(&share));
        assert_eq!(
            s.resolve_committed(&base, &with_sep).expect("recent"),
            share
        );
    }

    #[test]
    fn goto_opens_an_edit_on_the_focused_pane_with_the_current_path() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        let edit = s.path_edit.as_ref().expect("edit");
        assert_eq!(edit.text, native("/home/alice/"));
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
            vec![
                native("/home/alice/Documents/"),
                native("/home/alice/Downloads/")
            ],
            "no files, no dot dirs"
        );
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text(native("/home/alice/do"));
        }
        settle(&mut s);
        assert_eq!(texts(&s).len(), 2, "case-insensitive prefix");
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text(native("/home/alice/."));
        }
        settle(&mut s);
        assert_eq!(texts(&s), vec![native("/home/alice/.cache/")]);
    }

    #[test]
    fn relative_text_completes_against_the_pane_location() {
        let (mut s, _home) = session();
        s.dispatch("nav.goto", &Params::Null).expect("goto");
        if let Some(e) = s.path_edit.as_mut() {
            e.set_text("Docu".to_string());
        }
        settle(&mut s);
        let expected = if cfg!(windows) {
            "Documents\\"
        } else {
            "Documents/"
        };
        assert_eq!(
            texts(&s),
            vec![expected],
            "local Windows folders end in a backslash"
        );
    }

    #[test]
    fn history_is_recent_first_deduplicated_and_filtered() {
        let (mut s, home) = session();
        let docs = home.join("Documents").expect("uri");
        s.note_visit(&docs);
        s.note_visit(&home);
        s.note_visit(&docs);
        assert_eq!(s.path_history, vec![docs.clone(), home.clone()]);
        s.path_edit = Some(PathEdit::new(s.layout.focused(), native("/home/alice/")));
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
            s.path_edit.as_ref().is_some_and(|e| e
                .suggestions
                .iter()
                .any(|x| x.kind == SuggestionKind::History
                    && x.text == native("/home/alice/Documents"))),
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
        assert_eq!(s.path_history.first(), Some(&docs));

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
