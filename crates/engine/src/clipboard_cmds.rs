//! Cut / copy / paste of files (Ctrl+X / Ctrl+C / Ctrl+V).
//!
//! The clipboard is the session's own: it holds uris, and paste runs the ordinary copy or
//! move pipeline into the focused folder. The selection's paths are also mirrored to the OS
//! clipboard as text (via [`Session::clipboard_outbox`], written by the app); a paste whose
//! OS text no longer matches means something else was copied since, so nothing is pasted.

use fm_ops::ConflictPolicy;
use fm_vfs::{FmError, VfsUri};

use crate::file_cmds::{focused_dir, focused_selection, object};
use crate::fileops::OpRequest;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

#[derive(Debug, Clone)]
pub struct Clipboard {
    pub sources: Vec<VfsUri>,
    /// Cut (move on paste) rather than copy.
    pub cut: bool,
    /// What was mirrored to the OS clipboard.
    pub text: String,
}

fn store(s: &mut Session, cut: bool) {
    let sources = focused_selection(s);
    if sources.is_empty() {
        return;
    }
    let text = sources
        .iter()
        .map(|u| {
            u.local_path_string()
                .unwrap_or_else(|| u.path_str().to_string())
        })
        .collect::<Vec<_>>()
        .join("\n");
    s.clipboard_outbox = Some(text.clone());
    let verb = if cut { "Cut" } else { "Copied" };
    s.info = Some((
        format!("{verb} {}", plural(sources.len())),
        std::time::Instant::now(),
    ));
    s.clipboard = Some(Clipboard { sources, cut, text });
}

fn plural(n: usize) -> String {
    if n == 1 {
        "1 item".to_string()
    } else {
        format!("{n} items")
    }
}

fn can_paste(s: &Session) -> bool {
    s.clipboard.is_some() && focused_dir(s).is_some()
}

/// `file.clip_copy` — remember the selection for a later paste.
pub fn copy() -> CommandSpec {
    CommandSpec {
        id: "file.clip_copy",
        label: "Copy",
        menu: &["Edit", "Copy"],
        shortcut: Some("Ctrl+C"),
        params: "none",
        enabled: |s| !focused_selection(s).is_empty(),
        run: |s, _p: &Params| {
            store(s, false);
            Ok(())
        },
    }
}

/// `file.clip_cut` — remember the selection; the paste moves it.
pub fn cut() -> CommandSpec {
    CommandSpec {
        id: "file.clip_cut",
        label: "Cut",
        menu: &["Edit", "Cut"],
        shortcut: Some("Ctrl+X"),
        params: "none",
        enabled: |s| !focused_selection(s).is_empty(),
        run: |s, _p: &Params| {
            store(s, true);
            Ok(())
        },
    }
}

/// `file.clip_paste` — copy or move the remembered items into the focused folder.
/// `os_text` (from the OS paste event) must match what was mirrored, else nothing is pasted.
pub fn paste() -> CommandSpec {
    CommandSpec {
        id: "file.clip_paste",
        label: "Paste",
        menu: &["Edit", "Paste"],
        shortcut: Some("Ctrl+V"),
        params: "{os_text?: string, dest?: uri}",
        enabled: can_paste,
        run: |s, params: &Params| {
            let explicit_dest = object(params)?
                .and_then(|m| m.get("dest"))
                .map(crate::file_cmds::parse_uri)
                .transpose()?;
            let os_text = match object(params)?.and_then(|m| m.get("os_text")) {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| FmError::InvalidParams("os_text: string".to_string()))?,
                ),
                None => None,
            };
            let Some(clip) = s.clipboard.clone() else {
                return Ok(());
            };
            if os_text.is_some_and(|t| t != clip.text) {
                s.clipboard = None;
                return Err(FmError::InvalidParams(
                    "the clipboard changed outside the file manager; nothing to paste".to_string(),
                ));
            }
            let dest = explicit_dest
                .or_else(|| focused_dir(s))
                .ok_or_else(|| FmError::NotFound("a folder to paste into".to_string()))?;
            let count = plural(clip.sources.len());
            let policy = ConflictPolicy::Ask;
            let request = if clip.cut {
                OpRequest::Move {
                    sources: clip.sources,
                    dest,
                    policy,
                }
            } else {
                OpRequest::Duplicate {
                    sources: clip.sources,
                    dest,
                    policy,
                }
            };
            s.submit_op(request)?;
            s.info = Some((format!("Pasting {count}"), std::time::Instant::now()));
            if clip.cut {
                s.clipboard = None;
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![copy(), cut(), paste()]
}
