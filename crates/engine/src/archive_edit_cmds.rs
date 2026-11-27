//! Saving and discarding staged archive changes (`archive-editing`): `archive.save`,
//! `archive.save_as`, `archive.discard` and `archive.changes`, plus `archive.backup.restore`
//! and `archive.backup.discard` for the backup parts an interrupted split save leaves.
//! Saving runs as a file job (`archive_ops::save`).

use fm_vfs::{FmError, Scheme, VfsUri};
use serde_json::json;

use crate::archive_cmds::{MAX_PASSWORD, after_save, target_mount, uri_of};
use crate::archive_ops::SaveTarget;
use crate::archive_parts::BackupAction;
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
                backup: None,
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
            // A copy of a split archive is named by what the parts make up; the save cuts
            // it into parts of the original size (`name.7z.001`, …).
            let name = fm_archive::split::parse(name)
                .filter(|p| p.kind == fm_archive::split::SplitKind::Numbered)
                .map_or(name.to_string(), |p| p.base);
            let copy_to = dir.join(&name)?;
            s.submit_op(OpRequest::ArchiveSave(SaveTarget {
                mount,
                copy_to: Some(copy_to),
                backup: None,
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

/// The focused pane is inside a split archive that has backup parts of an interrupted save.
fn focused_has_backups(s: &Session) -> bool {
    s.layout
        .state(s.layout.focused())
        .and_then(|p| s.archives.for_uri(&p.location))
        .and_then(|m| m.opened())
        .and_then(|fs| fs.parts().map(|p| !p.backups.is_empty()))
        .unwrap_or(false)
}

/// The split archive a backup command acts on: its `uri` (a location inside the archive,
/// or any part file of the set), else the focused pane's archive.
fn backup_mount(
    s: &mut Session,
    params: &Params,
) -> Result<std::sync::Arc<fm_archive::mount::LazyArchive>, FmError> {
    match uri_of(params)? {
        Some(uri) if uri.archive_handle().is_none() => {
            let name = uri
                .file_name()
                .ok_or_else(|| FmError::InvalidParams("not a file".into()))?;
            let numbered = fm_archive::split::parse(name)
                .is_some_and(|p| p.kind == fm_archive::split::SplitKind::Numbered);
            if !numbered {
                return Err(FmError::InvalidParams(
                    "not a part of a split archive".into(),
                ));
            }
            let root = crate::archive_cmds::mount(s, &uri)?;
            s.archives
                .for_uri(&root)
                .cloned()
                .ok_or_else(|| FmError::InvalidParams("not inside an archive".into()))
        }
        _ => target_mount(s, params),
    }
}

fn backup_command(s: &mut Session, params: &Params, action: BackupAction) -> Result<(), FmError> {
    check_shape(params)?;
    let mount = backup_mount(s, params)?;
    if mount.is_dirty() {
        return Err(FmError::InvalidParams(
            "discard or save the staged changes first".into(),
        ));
    }
    s.submit_op(OpRequest::ArchiveSave(SaveTarget {
        mount,
        copy_to: None,
        backup: Some(action),
    }))
    .map(|_| ())
}

/// `archive.backup.restore {uri?}` — put back the parts a split save set aside when it
/// was interrupted (the saved changes in the new parts are dropped).
pub fn backup_restore() -> CommandSpec {
    CommandSpec {
        id: "archive.backup.restore",
        label: "Restore Previous Version",
        menu: &["File", "Archive", "Restore Previous Version"],
        shortcut: None,
        params: "{uri?: string} — a location inside the split archive, or one of its part files",
        enabled: focused_has_backups,
        run: |s, params: &Params| backup_command(s, params, BackupAction::Restore),
    }
}

/// `archive.backup.discard {uri?}` — delete the backup parts of an interrupted split save
/// and keep the parts now in place.
pub fn backup_discard() -> CommandSpec {
    CommandSpec {
        id: "archive.backup.discard",
        label: "Delete Backup Parts",
        menu: &["File", "Archive", "Delete Backup Parts"],
        shortcut: None,
        params: "{uri?: string} — a location inside the split archive, or one of its part files",
        enabled: focused_has_backups,
        run: |s, params: &Params| backup_command(s, params, BackupAction::Discard),
    }
}

/// The focused pane is inside a writable zip or 7z (they can carry a password).
fn focused_can_encrypt(s: &Session) -> bool {
    s.layout
        .state(s.layout.focused())
        .and_then(|p| s.archives.for_uri(&p.location))
        .and_then(|m| m.opened())
        .is_some_and(|fs| {
            fs.writable().is_ok()
                && matches!(
                    fs.format(),
                    fm_archive::FormatId::Zip | fm_archive::FormatId::SevenZ
                )
        })
}

fn stage_password(
    s: &mut Session,
    params: &Params,
    new: Option<fm_archive::Secret>,
) -> Result<(), FmError> {
    let mount = target_mount(s, params)?;
    let fs = mount
        .opened()
        .ok_or_else(|| FmError::InvalidParams("the archive is not open".into()))?;
    let encrypt_names = match field(params, "encrypt_names") {
        None | Some(Params::Null) => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| FmError::InvalidParams("encrypt_names must be true or false".into()))?,
    };
    fs.stage_password(fm_archive::overlay::PasswordChange { new, encrypt_names })?;
    let root = mount.root().clone();
    after_save(s, &root);
    Ok(())
}

/// `archive.password.set {uri?, password?, encrypt_names?}` — stage a new password for
/// a zip or 7z (applied on Save: every member is encrypted with AES-256; 7z hides the
/// names unless `encrypt_names` is false). Without `password` the password is removed.
pub fn password_set() -> CommandSpec {
    CommandSpec {
        id: "archive.password.set",
        label: "Set Password…",
        menu: &["File", "Archive", "Set Password…"],
        shortcut: None,
        params: "{uri?: string, password?: string (1–1024 bytes; absent removes it), encrypt_names?: bool (7z, default true)}",
        enabled: focused_can_encrypt,
        run: |s, params: &Params| {
            check_shape(params)?;
            let new = match field(params, "password") {
                None | Some(Params::Null) => None,
                Some(Params::String(pw)) if !pw.is_empty() && pw.len() <= MAX_PASSWORD => {
                    Some(fm_archive::Secret::new(pw.clone()))
                }
                Some(_) => {
                    return Err(FmError::InvalidParams(format!(
                        "password must be 1 to {MAX_PASSWORD} bytes"
                    )));
                }
            };
            stage_password(s, params, new)
        },
    }
}

/// `archive.password.remove {uri?}` — stage removing the password (applied on Save).
pub fn password_remove() -> CommandSpec {
    CommandSpec {
        id: "archive.password.remove",
        label: "Remove Password",
        menu: &["File", "Archive", "Remove Password"],
        shortcut: None,
        params: "{uri?: string}",
        enabled: |s| {
            focused_can_encrypt(s)
                && s.layout
                    .state(s.layout.focused())
                    .and_then(|p| s.archives.for_uri(&p.location))
                    .and_then(|m| m.opened())
                    .is_some_and(|fs| {
                        let info = fs.info();
                        info.encrypted || info.names_encrypted
                    })
        },
        run: |s, params: &Params| {
            check_shape(params)?;
            stage_password(s, params, None)
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        save(),
        save_as(),
        discard(),
        changes(),
        password_set(),
        password_remove(),
        backup_restore(),
        backup_discard(),
    ]
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
            "archive.password.set",
            "archive.password.remove",
            "archive.backup.restore",
            "archive.backup.discard",
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
