//! Saving and discarding staged archive changes (`archive-editing`): `archive.save`,
//! `archive.save_as`, `archive.discard` and `archive.changes`. Saving runs as a file job
//! (`archive_ops::save`).

use fm_vfs::{FmError, Scheme, VfsUri};
use serde_json::json;

use crate::archive_cmds::{after_save, target_mount, uri_of};
use crate::archive_ops::SaveTarget;
use crate::fileops::OpRequest;
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// The focused pane is inside an archive with unsaved staged changes.
fn focused_dirty(s: &Session) -> bool {
    s.layout
        .state(s.layout.focused())
        .filter(|p| p.location.scheme() == Scheme::Archive)
        .and_then(|p| s.archives.for_uri(&p.location))
        .is_some_and(|m| m.is_dirty())
}

fn focused_in_archive(s: &Session) -> bool {
    s.layout
        .state(s.layout.focused())
        .and_then(|p| s.archives.for_uri(&p.location))
        .is_some_and(|m| m.opened().is_some_and(|fs| fs.writable().is_ok()))
}

fn field<'a>(params: &'a Params, key: &str) -> Option<&'a Params> {
    params.as_object().and_then(|m| m.get(key))
}

fn check_shape(params: &Params) -> Result<(), FmError> {
    match params {
        Params::Null | Params::Object(_) | Params::String(_) => {
            uri_of(params)?;
            Ok(())
        }
        _ => Err(FmError::InvalidParams("expected {uri?: string}".into())),
    }
}

/// `archive.save {uri?}` — write the archive with its staged changes.
pub fn save() -> CommandSpec {
    CommandSpec {
        id: "archive.save",
        label: "Save Archive",
        menu: &["File", "Archive", "Save Archive"],
        shortcut: None,
        params: "{uri?: string} — a location inside the archive; default the focused pane's",
        enabled: focused_dirty,
        run: |s, params: &Params| {
            check_shape(params)?;
            let mount = target_mount(s, params)?;
            let fs = mount.opened().filter(|fs| fs.is_dirty()).ok_or_else(|| {
                FmError::InvalidParams("the archive has no unsaved changes".into())
            })?;
            fs.writable()?;
            s.submit_op(OpRequest::ArchiveSave(SaveTarget {
                mount,
                copy_to: None,
            }))
            .map(|_| ())
        },
    }
}

/// `archive.save_as {uri?, dest?}` — write the archive with its staged changes to `dest`
/// (a folder; the copy keeps the archive's name, numbered when taken). Without `dest`,
/// the path picker asks for the folder.
pub fn save_as() -> CommandSpec {
    CommandSpec {
        id: "archive.save_as",
        label: "Save Archive as Copy…",
        menu: &["File", "Archive", "Save Archive as Copy…"],
        shortcut: None,
        params: "{uri?: string, dest?: string} — dest is a folder; without it the picker asks",
        enabled: focused_in_archive,
        run: |s, params: &Params| {
            check_shape(params)?;
            let mount = target_mount(s, params)?;
            mount
                .opened()
                .ok_or_else(|| FmError::InvalidParams("the archive is not open".into()))?
                .writable()?;
            let dest = match field(params, "dest") {
                None | Some(Params::Null) => None,
                Some(Params::String(raw)) => {
                    Some(VfsUri::parse(raw).map_err(|_| FmError::invalid_uri(raw.to_string()))?)
                }
                Some(_) => return Err(FmError::InvalidParams("dest must be a folder uri".into())),
            };
            let Some(dir) = dest else {
                let start = mount
                    .container()
                    .parent()
                    .filter(|p| p.archive_handle().is_none())
                    .map(|p| p.to_string());
                s.archives.save_as_pending = Some(mount.root().clone());
                return s.dispatch(
                    "picker.open",
                    &json!({
                        "purpose": "archive.save_as",
                        "kind": "folder",
                        "title": "Save a copy of the archive in which folder?",
                        "start": start,
                    }),
                );
            };
            if dir.archive_handle() == mount.root().archive_handle() {
                return Err(FmError::InvalidParams(
                    "a copy cannot be saved inside the archive itself".into(),
                ));
            }
            let name = mount
                .container()
                .file_name()
                .ok_or_else(|| FmError::InvalidParams("the archive has no name".into()))?;
            let copy_to = dir.join(name)?;
            s.submit_op(OpRequest::ArchiveSave(SaveTarget {
                mount,
                copy_to: Some(copy_to),
            }))
            .map(|_| ())
        },
    }
}

/// `archive.discard {uri?, confirm: true}` — drop every staged change of the archive.
pub fn discard() -> CommandSpec {
    CommandSpec {
        id: "archive.discard",
        label: "Discard Archive Changes",
        menu: &["File", "Archive", "Discard Archive Changes"],
        shortcut: None,
        params: "{uri?: string, confirm: true} — the caller must have confirmed",
        enabled: focused_dirty,
        run: |s, params: &Params| {
            check_shape(params)?;
            if field(params, "confirm").and_then(Params::as_bool) != Some(true) {
                return Err(FmError::InvalidParams(
                    "discarding changes requires confirm: true".into(),
                ));
            }
            let mount = target_mount(s, params)?;
            let fs = mount.opened().filter(|fs| fs.is_dirty()).ok_or_else(|| {
                FmError::InvalidParams("the archive has no unsaved changes".into())
            })?;
            if fs.is_saving() {
                return Err(FmError::InvalidParams("the archive is being saved".into()));
            }
            fs.discard();
            let root = mount.root().clone();
            after_save(s, &root);
            Ok(())
        },
    }
}

/// `archive.changes {uri?}` — show the list of the archive's staged changes.
pub fn changes() -> CommandSpec {
    CommandSpec {
        id: "archive.changes",
        label: "Show Archive Changes…",
        menu: &["File", "Archive", "Show Archive Changes…"],
        shortcut: None,
        params: "{uri?: string}",
        enabled: focused_dirty,
        run: |s, params: &Params| {
            check_shape(params)?;
            let mount = target_mount(s, params)?;
            if !mount.is_dirty() {
                return Err(FmError::InvalidParams(
                    "the archive has no unsaved changes".into(),
                ));
            }
            s.archives.changes_shown = Some(mount.root().clone());
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![save(), save_as(), discard(), changes()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_to_save_or_outside_an_archive_is_an_error() {
        let mut s = Session::new().expect("session");
        for id in [
            "archive.save",
            "archive.save_as",
            "archive.discard",
            "archive.changes",
        ] {
            assert!(s.dispatch(id, &json!({})).is_err(), "{id}");
            assert!(s.dispatch(id, &json!({"uri": "/tmp"})).is_err(), "{id}");
            assert!(s.dispatch(id, &json!(7)).is_err(), "{id}");
            assert!(s.dispatch(id, &json!({"uri": 7})).is_err(), "{id}");
        }
        assert!(s.dispatch("archive.save_as", &json!({"dest": 3})).is_err());
        assert!(
            s.dispatch("archive.discard", &json!({"confirm": false}))
                .is_err(),
            "unconfirmed"
        );
    }
}
