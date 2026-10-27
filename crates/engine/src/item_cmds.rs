//! Item commands: Copy Path, Copy Name and Duplicate for the focused selection.

use fm_ops::ConflictPolicy;
use fm_vfs::{FmError, Scheme, VfsUri};

use crate::file_cmds::{focused_dir, focused_selection, object, parse_sources};
use crate::fileops::OpRequest;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// What the user would type to reach `uri`: a plain path for local items (`C:\x` on
/// Windows), the full uri for remote ones.
pub fn display_path(uri: &VfsUri) -> String {
    if uri.scheme() != Scheme::File {
        return uri.to_string();
    }
    if uri.is_drive_list() {
        return "This PC".to_string();
    }
    uri.local_path_string()
        .unwrap_or_else(|| uri.path_str().to_string())
}

/// The selection, or the folder itself when nothing is selected.
fn targets(s: &Session, params: &Params) -> Result<Vec<VfsUri>, FmError> {
    let map = object(params)?;
    if let Some(v) = map.and_then(|m| m.get("sources")) {
        return parse_sources(v);
    }
    let selection = focused_selection(s);
    if !selection.is_empty() {
        return Ok(selection);
    }
    Ok(focused_dir(s).into_iter().collect())
}

fn copy_text(s: &mut Session, items: Vec<String>, what: &str) {
    if items.is_empty() {
        return;
    }
    let message = match items.as_slice() {
        [one] => format!("Copied {what}: {one}"),
        many => format!("Copied {} {what}s", many.len()),
    };
    s.clipboard_out = Some(items.join("\n"));
    s.info = Some((message, std::time::Instant::now()));
}

fn has_target(s: &Session) -> bool {
    !focused_selection(s).is_empty() || focused_dir(s).is_some()
}

pub fn copy_path() -> CommandSpec {
    CommandSpec {
        id: "file.copy_path",
        label: "Copy Path",
        menu: &["Edit", "Copy Path"],
        shortcut: None,
        params: "sources?: [uri] (default: the selection or else the folder)",
        enabled: has_target,
        run: |s, params| {
            let items = targets(s, params)?.iter().map(display_path).collect();
            copy_text(s, items, "path");
            Ok(())
        },
    }
}

pub fn copy_name() -> CommandSpec {
    CommandSpec {
        id: "file.copy_name",
        label: "Copy Name",
        menu: &["Edit", "Copy Name"],
        shortcut: None,
        params: "sources?: [uri] (default: the selection or else the folder)",
        enabled: has_target,
        run: |s, params| {
            let items = targets(s, params)?
                .iter()
                .map(|u| {
                    u.file_name()
                        .map(str::to_string)
                        .unwrap_or_else(|| display_path(u))
                })
                .collect();
            copy_text(s, items, "name");
            Ok(())
        },
    }
}

/// `file.duplicate` — copy each item next to itself as `name (2).ext`.
pub fn duplicate() -> CommandSpec {
    CommandSpec {
        id: "file.duplicate",
        label: "Duplicate",
        menu: &["File", "Duplicate"],
        shortcut: Some("Ctrl+Shift+D"),
        params: "sources?: [uri] (default: selection)",
        enabled: |s| s.focused_remote_ready() && !focused_selection(s).is_empty(),
        run: |s, params| {
            let map = object(params)?;
            let sources = match map.and_then(|m| m.get("sources")) {
                Some(v) => parse_sources(v)?,
                None => focused_selection(s),
            };
            // One job per parent folder (a search result set may span several).
            let mut groups: Vec<(VfsUri, Vec<VfsUri>)> = Vec::new();
            for src in sources {
                let parent = src
                    .parent()
                    .ok_or_else(|| FmError::InvalidParams("cannot duplicate a root".into()))?;
                match groups.iter_mut().find(|(p, _)| *p == parent) {
                    Some((_, list)) => list.push(src),
                    None => groups.push((parent, vec![src])),
                }
            }
            for (dest, sources) in groups {
                s.submit_op(OpRequest::Duplicate {
                    sources,
                    dest,
                    policy: ConflictPolicy::Ask,
                })?;
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![copy_path(), copy_name(), duplicate()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn copy_path_and_name_fill_the_clipboard_outbox() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "file.copy_path",
            &json!({"sources": ["/home/u/a b.txt", "sftp://h/srv/x"]}),
        )
        .expect("path");
        let text = s.clipboard_out.take().expect("text");
        if cfg!(windows) {
            assert!(text.contains("\\home\\u\\a b.txt"));
        } else {
            assert_eq!(text, "/home/u/a b.txt\nsftp://h/srv/x");
        }
        s.dispatch("file.copy_name", &json!({"sources": ["/home/u/a b.txt"]}))
            .expect("name");
        assert_eq!(s.clipboard_out.take().as_deref(), Some("a b.txt"));
        // Nothing selected: the focused folder itself.
        s.dispatch("file.copy_path", &Params::Null).expect("dir");
        assert!(s.clipboard_out.take().is_some());
    }

    #[test]
    fn duplicate_numbers_the_copy_next_to_the_original() {
        let tmp = tempfile::tempdir().expect("tmp");
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"hi").expect("write");
        let mut s = Session::new().expect("session");
        let uri = VfsUri::parse(file.to_str().expect("utf8")).expect("uri");
        s.dispatch("file.duplicate", &json!({"sources": [uri.as_str()]}))
            .expect("dup");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !tmp.path().join("a (2).txt").exists() && std::time::Instant::now() < deadline {
            s.poll();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            std::fs::read(tmp.path().join("a (2).txt")).expect("copy"),
            b"hi"
        );
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for (id, p) in [
            ("file.copy_path", json!({"sources": "x"})),
            ("file.copy_name", json!({"sources": [1]})),
            ("file.duplicate", json!({"sources": ["/"]})),
            ("file.duplicate", json!("x")),
        ] {
            assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
        }
    }
}
