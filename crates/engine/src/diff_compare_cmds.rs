//! "Compare with…": pick a second file (any backend) for the selected one and show the two as
//! a diff in a Preview pane (capability `git-diff`).

use fm_layout::PaneKind;
use fm_vfs::{FmError, Scheme, VfsUri};
use serde_json::json;

use crate::file_cmds::{focused_selection, object, parse_uri};
use crate::registry::{CommandSpec, Params};

/// The file chooser opened by `file.compare_with`.
#[derive(Debug, Clone)]
pub struct ComparePicker {
    /// The file being compared.
    pub source: VfsUri,
    /// The folder the chooser lists.
    pub folder: VfsUri,
    pub selected: Option<VfsUri>,
}

fn comparable(uri: &VfsUri) -> bool {
    matches!(uri.scheme(), Scheme::File | Scheme::Sftp | Scheme::Smb)
}

fn uri_param(params: &Params, key: &str) -> Result<Option<VfsUri>, FmError> {
    object(params)?
        .and_then(|m| m.get(key))
        .filter(|v| !v.is_null())
        .map(parse_uri)
        .transpose()
}

fn open_dialog() -> CommandSpec {
    CommandSpec {
        id: "file.compare_with",
        label: "Compare With…",
        menu: &["File", "Compare With…"],
        shortcut: None,
        params: "{uri?: string} (default: the selected file)",
        enabled: |s| {
            let selection = focused_selection(s);
            selection.len() == 1 && selection.iter().all(comparable)
        },
        run: |s, params| {
            let uri = match uri_param(params, "uri")? {
                Some(uri) => uri,
                None => focused_selection(s)
                    .into_iter()
                    .next()
                    .ok_or_else(|| FmError::InvalidParams("select a file first".into()))?,
            };
            if !comparable(&uri) {
                return Err(FmError::Unsupported("this location cannot be compared"));
            }
            s.backends.require(&uri)?;
            let folder = uri
                .parent()
                .ok_or(FmError::Unsupported("cannot compare this location"))?;
            s.ensure_read(folder.clone())?;
            s.compare = Some(ComparePicker {
                source: uri,
                folder,
                selected: None,
            });
            Ok(())
        },
    }
}

fn navigate() -> CommandSpec {
    CommandSpec {
        id: "file.compare_with.navigate",
        label: "Browse Compare Folder",
        menu: &["File", "Browse Compare Folder"],
        shortcut: None,
        params: "{uri: folder to list}",
        enabled: |s| s.compare.is_some(),
        run: |s, params| {
            let folder = uri_param(params, "uri")?
                .ok_or_else(|| FmError::InvalidParams("uri: required".into()))?;
            if !comparable(&folder) {
                return Err(FmError::Unsupported("this location cannot be compared"));
            }
            s.backends.require(&folder)?;
            s.ensure_read(folder.clone())?;
            let picker = s
                .compare
                .as_mut()
                .ok_or(FmError::Unsupported("no comparison is being set up"))?;
            picker.folder = folder;
            picker.selected = None;
            Ok(())
        },
    }
}

fn select() -> CommandSpec {
    CommandSpec {
        id: "file.compare_with.select",
        label: "Choose Compare File",
        menu: &["File", "Choose Compare File"],
        shortcut: None,
        params: "{uri: the file to compare with}",
        enabled: |s| s.compare.is_some(),
        run: |s, params| {
            let uri = uri_param(params, "uri")?
                .ok_or_else(|| FmError::InvalidParams("uri: required".into()))?;
            let picker = s
                .compare
                .as_mut()
                .ok_or(FmError::Unsupported("no comparison is being set up"))?;
            picker.selected = Some(uri);
            Ok(())
        },
    }
}

fn close() -> CommandSpec {
    CommandSpec {
        id: "file.compare_with.close",
        label: "Close Compare With",
        menu: &["File", "Close Compare With"],
        shortcut: None,
        params: "none",
        enabled: |s| s.compare.is_some(),
        run: |s, params| {
            object(params)?;
            s.compare = None;
            Ok(())
        },
    }
}

fn compare() -> CommandSpec {
    CommandSpec {
        id: "preview.diff.compare",
        label: "Compare Files",
        menu: &["View", "Preview Diff", "Compare Files"],
        shortcut: None,
        params: "{a?: string, b?: string} (default: the Compare With… source and chosen file)",
        enabled: |s| s.compare.as_ref().is_some_and(|c| c.selected.is_some()),
        run: |s, params| {
            let a =
                uri_param(params, "a")?.or_else(|| s.compare.as_ref().map(|c| c.source.clone()));
            let b = uri_param(params, "b")?
                .or_else(|| s.compare.as_ref().and_then(|c| c.selected.clone()));
            let (Some(a), Some(b)) = (a, b) else {
                return Err(FmError::InvalidParams("a and b are required".into()));
            };
            if !comparable(&a) || !comparable(&b) {
                return Err(FmError::Unsupported("this location cannot be compared"));
            }
            if a == b {
                return Err(FmError::InvalidParams(
                    "choose a different file to compare with".into(),
                ));
            }
            s.backends.require(&b)?;
            s.dispatch("preview.open", &json!({ "uri": a.to_string() }))?;
            let origin = s.layout.focused();
            let pane = s
                .layout
                .panes()
                .into_iter()
                .filter(|id| s.pane_kind(*id) == Some(PaneKind::Preview))
                .find(|id| s.layout.preview_target(*id) == Some(origin))
                .ok_or(FmError::NotFound("no preview pane".into()))?;
            s.pin_pair(pane, a, b);
            s.compare = None;
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open_dialog(), navigate(), select(), close(), compare()]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::session::Session;

    fn session() -> Session {
        Session::new().expect("session")
    }

    #[test]
    fn malformed_params_are_errors_not_panics() {
        let mut s = session();
        for (id, params) in [
            ("file.compare_with", json!({"uri": 5})),
            ("file.compare_with", json!({"uri": "sftp://nowhere/x"})),
            ("file.compare_with.navigate", json!({})),
            ("file.compare_with.select", json!({"uri": 1})),
            ("file.compare_with.close", json!([1])),
            ("preview.diff.compare", json!({"a": "x"})),
            ("preview.diff.compare", json!({"a": 1, "b": 2})),
        ] {
            assert!(s.dispatch(id, &params).is_err(), "{id} {params}");
        }
        assert!(s.compare.is_none());
    }

    #[test]
    fn comparing_a_file_with_itself_is_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").expect("write");
        let uri = VfsUri::parse(&file.to_string_lossy()).expect("uri");
        let mut s = session();
        let err = s
            .dispatch(
                "preview.diff.compare",
                &json!({"a": uri.to_string(), "b": uri.to_string()}),
            )
            .expect_err("same file");
        assert!(matches!(err, FmError::InvalidParams(_)));
    }

    #[test]
    fn dialog_lists_the_source_folder_and_closes() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("a.txt");
        std::fs::write(&file, b"x").expect("write");
        let uri = VfsUri::parse(&file.to_string_lossy()).expect("uri");
        let mut s = session();
        s.dispatch("file.compare_with", &json!({"uri": uri.to_string()}))
            .expect("open");
        let picker = s.compare.clone().expect("picker");
        assert_eq!(picker.source, uri);
        assert_eq!(Some(picker.folder), uri.parent());
        s.dispatch("file.compare_with.close", &Params::Null)
            .expect("close");
        assert!(s.compare.is_none());
    }
}
