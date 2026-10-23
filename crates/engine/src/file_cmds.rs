//! File commands: copy, move, mkdir, trash, permanent delete, rename, undo.
//!
//! Every command validates its (untrusted) params synchronously and then enqueues a job
//! via [`Session::submit_op`]; no filesystem work happens on the calling thread.

use serde_json::Map;

use fm_ops::{ConflictPolicy, validate_name};
use fm_vfs::{FmError, VfsUri};

use crate::fileops::{MAX_SOURCES, OpRequest};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

pub(crate) type Obj = Map<String, Params>;

/// `null` and `{}` both mean "use the defaults"; anything else non-object is an error.
pub(crate) fn object(params: &Params) -> Result<Option<&Obj>, FmError> {
    match params {
        Params::Null => Ok(None),
        Params::Object(map) => Ok(Some(map)),
        _ => Err(FmError::InvalidParams(
            "expected an object of named parameters".to_string(),
        )),
    }
}

pub(crate) fn parse_uri(value: &Params) -> Result<VfsUri, FmError> {
    let raw = value
        .as_str()
        .ok_or_else(|| FmError::InvalidParams("expected a uri string".to_string()))?;
    VfsUri::parse(raw).map_err(|_| FmError::invalid_uri(raw.to_string()))
}

pub(crate) fn parse_sources(value: &Params) -> Result<Vec<VfsUri>, FmError> {
    let list = value
        .as_array()
        .ok_or_else(|| FmError::InvalidParams("sources must be an array of uris".to_string()))?;
    if list.len() > MAX_SOURCES {
        return Err(FmError::InvalidParams(format!(
            "too many sources (limit {MAX_SOURCES})"
        )));
    }
    list.iter().map(parse_uri).collect()
}

fn parse_policy(map: Option<&Obj>) -> Result<ConflictPolicy, FmError> {
    let Some(value) = map.and_then(|m| m.get("policy")) else {
        return Ok(ConflictPolicy::Ask);
    };
    match value.as_str() {
        Some("ask") => Ok(ConflictPolicy::Ask),
        Some("overwrite") => Ok(ConflictPolicy::Overwrite),
        Some("keep_both") => Ok(ConflictPolicy::KeepBoth),
        Some("skip") => Ok(ConflictPolicy::Skip),
        _ => Err(FmError::InvalidParams(
            "policy must be ask, overwrite, keep_both or skip".to_string(),
        )),
    }
}

fn required_name(map: Option<&Obj>) -> Result<String, FmError> {
    let name = map
        .and_then(|m| m.get("name"))
        .and_then(Params::as_str)
        .ok_or_else(|| FmError::InvalidParams("name: string is required".to_string()))?;
    validate_name(name)?;
    Ok(name.to_string())
}

/// Live validation for the mkdir/rename dialogs: a legal name that does not already exist
/// in `dir` (per the cached listing; `current` is the item being renamed). Pure — no I/O.
pub fn validate_new_name(
    s: &Session,
    dir: &VfsUri,
    name: &str,
    current: Option<&str>,
) -> Result<(), String> {
    validate_name(name).map_err(|_| "Not a valid name".to_string())?;
    if current == Some(name) {
        return Ok(());
    }
    let exists = s
        .dirs
        .entries(dir)
        .is_some_and(|entries| entries.iter().any(|e| e.name == name));
    if exists {
        return Err("Already exists".to_string());
    }
    Ok(())
}

/// Uris of the focused pane's selected rows.
pub fn focused_selection(s: &Session) -> Vec<VfsUri> {
    let Some(pane) = s.layout.state(s.layout.focused()) else {
        return Vec::new();
    };
    pane.listing
        .selection()
        .filter_map(|name| pane.location.join(name).ok())
        .collect()
}

/// Directory of the most-recently-focused other pane (the copy/move destination).
pub fn other_pane_dir(s: &Session) -> Option<VfsUri> {
    let focused = s.layout.focused();
    let other = s
        .layout
        .focus
        .previous
        .filter(|p| *p != focused && s.layout.state(*p).is_some())
        .or_else(|| s.layout.panes().into_iter().find(|p| *p != focused))?;
    s.layout.state(other).map(|st| st.location.clone())
}

fn sources_or_selection(s: &Session, map: Option<&Obj>) -> Result<Vec<VfsUri>, FmError> {
    match map.and_then(|m| m.get("sources")) {
        Some(value) => parse_sources(value),
        None => Ok(focused_selection(s)),
    }
}

fn has_selection(s: &Session) -> bool {
    !focused_selection(s).is_empty()
}

fn transfer(s: &mut Session, params: &Params, is_move: bool) -> Result<(), FmError> {
    let map = object(params)?;
    let policy = parse_policy(map)?;
    let sources = sources_or_selection(s, map)?;
    if sources.is_empty() {
        return Ok(());
    }
    let dest = match map.and_then(|m| m.get("dest")) {
        Some(value) => parse_uri(value)?,
        None => other_pane_dir(s).ok_or_else(|| {
            FmError::InvalidParams("no destination: open a second pane or pass `dest`".to_string())
        })?,
    };
    let request = if is_move {
        OpRequest::Move {
            sources,
            dest,
            policy,
        }
    } else {
        OpRequest::Copy {
            sources,
            dest,
            policy,
        }
    };
    s.submit_op(request).map(|_| ())
}

/// `file.copy` — copy the selection into the other pane's directory.
pub fn copy() -> CommandSpec {
    CommandSpec {
        id: "file.copy",
        label: "Copy to Other Pane",
        menu: &["File", "Copy"],
        shortcut: Some("F5"),
        params: "sources?: [uri], dest?: uri, policy?: ask|overwrite|keep_both|skip",
        enabled: has_selection,
        run: |s, params| transfer(s, params, false),
    }
}

/// `file.move` — move the selection into the other pane's directory.
pub fn move_() -> CommandSpec {
    CommandSpec {
        id: "file.move",
        label: "Move to Other Pane",
        menu: &["File", "Move"],
        shortcut: Some("F6"),
        params: "sources?: [uri], dest?: uri, policy?: ask|overwrite|keep_both|skip",
        enabled: has_selection,
        run: |s, params| transfer(s, params, true),
    }
}

/// `file.mkdir` — create a folder in the focused pane (or `parent`).
pub fn mkdir() -> CommandSpec {
    CommandSpec {
        id: "file.mkdir",
        label: "New Folder",
        menu: &["File", "New Folder"],
        shortcut: Some("F7"),
        params: "name: string, parent?: uri",
        enabled: |s| s.layout.state(s.layout.focused()).is_some(),
        run: |s, params| {
            let map = object(params)?;
            let name = required_name(map)?;
            let parent = match map.and_then(|m| m.get("parent")) {
                Some(value) => parse_uri(value)?,
                None => s
                    .layout
                    .state(s.layout.focused())
                    .map(|p| p.location.clone())
                    .ok_or_else(|| FmError::NotFound("focused pane".to_string()))?,
            };
            s.submit_op(OpRequest::Mkdir { parent, name }).map(|_| ())
        },
    }
}

/// `file.rename` — rename the single selected item (or `uri`).
pub fn rename() -> CommandSpec {
    CommandSpec {
        id: "file.rename",
        label: "Rename",
        menu: &["File", "Rename"],
        shortcut: Some("F2"),
        params: "name: string, uri?: uri",
        enabled: |s| focused_selection(s).len() == 1,
        run: |s, params| {
            let map = object(params)?;
            let name = required_name(map)?;
            let uri = match map.and_then(|m| m.get("uri")) {
                Some(value) => parse_uri(value)?,
                None => {
                    let mut selected = focused_selection(s);
                    match selected.len() {
                        0 => return Ok(()),
                        1 => selected.remove(0),
                        _ => {
                            return Err(FmError::InvalidParams(
                                "rename needs exactly one selected item".to_string(),
                            ));
                        }
                    }
                }
            };
            s.submit_op(OpRequest::Rename { uri, name }).map(|_| ())
        },
    }
}

/// `file.delete` — move the selection to the platform trash.
pub fn delete() -> CommandSpec {
    CommandSpec {
        id: "file.delete",
        label: "Move to Trash",
        menu: &["File", "Move to Trash"],
        shortcut: Some("F8"),
        params: "sources?: [uri]",
        enabled: has_selection,
        run: |s, params| {
            let map = object(params)?;
            let sources = sources_or_selection(s, map)?;
            if sources.is_empty() {
                return Ok(());
            }
            s.submit_op(OpRequest::Trash { sources }).map(|_| ())
        },
    }
}

/// `file.delete_permanent` — irreversible delete; the caller must have confirmed.
pub fn delete_permanent() -> CommandSpec {
    CommandSpec {
        id: "file.delete_permanent",
        label: "Delete Permanently",
        menu: &["File", "Delete Permanently"],
        shortcut: Some("Shift+Delete"),
        params: "confirm: true, sources?: [uri]",
        enabled: has_selection,
        run: |s, params| {
            let map = object(params)?;
            let confirmed = map
                .and_then(|m| m.get("confirm"))
                .and_then(Params::as_bool)
                .unwrap_or(false);
            if !confirmed {
                return Err(FmError::InvalidParams(
                    "permanent delete requires confirm: true".to_string(),
                ));
            }
            let sources = sources_or_selection(s, map)?;
            if sources.is_empty() {
                return Ok(());
            }
            s.submit_op(OpRequest::DeletePermanent { sources })
                .map(|_| ())
        },
    }
}

/// `edit.undo` — replay the newest inverse record as a job.
pub fn undo() -> CommandSpec {
    CommandSpec {
        id: "edit.undo",
        label: "Undo",
        menu: &["Edit", "Undo"],
        shortcut: Some("Ctrl+Z"),
        params: "none",
        enabled: |s| !s.undo.is_empty(),
        run: |s, _p: &Params| {
            let Some(record) = s.undo.pop() else {
                return Ok(());
            };
            s.submit_op(OpRequest::Undo(record)).map(|_| ())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        copy(),
        move_(),
        mkdir(),
        rename(),
        delete(),
        delete_permanent(),
        undo(),
    ]
}
