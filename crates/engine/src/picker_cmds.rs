//! The path picker: choose a folder or a file on any backend. The UI window only dispatches
//! `picker.*` commands; listings come from the directory cache like every other folder.

use fm_vfs::{FmError, Scheme, VfsUri};
use serde_json::json;

use crate::file_cmds::{object, parse_uri};
use crate::places_cmds::{Place, PlaceKind};
use crate::registry::{CommandSpec, Params};

/// What the picker returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickKind {
    Folder,
    File,
}

/// An open picker window.
#[derive(Debug, Clone)]
pub struct Picker {
    /// Who asked: `sync.left`, `sync.right`, or a dialog's own key (result in `Session::picked`).
    pub purpose: String,
    pub kind: PickKind,
    pub title: String,
    /// The folder being listed.
    pub folder: VfsUri,
    pub selected: Option<VfsUri>,
    /// Opened from a dialog that is already showing (a sign-in form's key file): the window
    /// must stay above that dialog.
    pub nested: bool,
}

const MAX_TEXT: usize = 4096;
/// Choosers that can wait beneath the open one.
const MAX_PARKED: usize = 4;

fn supported(uri: &VfsUri) -> bool {
    matches!(uri.scheme(), Scheme::File | Scheme::Sftp | Scheme::Smb)
}

fn text<'a>(params: &'a Params, key: &str) -> Result<Option<&'a str>, FmError> {
    match object(params)?
        .and_then(|m| m.get(key))
        .filter(|v| !v.is_null())
    {
        None => Ok(None),
        Some(v) => {
            let s = v
                .as_str()
                .ok_or_else(|| FmError::InvalidParams(format!("{key}: expected a string")))?;
            if s.len() > MAX_TEXT {
                return Err(FmError::InvalidParams(format!("{key}: too long")));
            }
            Ok(Some(s))
        }
    }
}

/// Where a result can be chosen: a real folder, not the Network or Saved servers lists.
pub fn choosable(uri: &VfsUri) -> bool {
    supported(uri) && !uri.is_drive_list()
}

fn browsable(uri: &VfsUri) -> bool {
    supported(uri) || matches!(uri.scheme(), Scheme::Network | Scheme::Servers)
}

/// The shortcuts beside the list: `(section, places)`; empty sections are left out.
pub fn places(s: &crate::Session) -> Vec<(&'static str, Vec<Place>)> {
    let place = |name: String, uri: VfsUri, kind, bookmark| Place {
        name,
        uri,
        bookmark,
        kind,
    };
    let all = s.places();
    let mut out = vec![
        (
            "Places",
            all.iter()
                .filter(|p| !p.bookmark)
                .cloned()
                .collect::<Vec<_>>(),
        ),
        (
            "Bookmarks",
            all.iter().filter(|p| p.bookmark).cloned().collect(),
        ),
        (
            "Devices",
            s.volumes
                .entries()
                .iter()
                .map(|d| {
                    place(
                        crate::devices::display_label(&d.name),
                        d.mount.clone(),
                        PlaceKind::Computer,
                        false,
                    )
                })
                .collect(),
        ),
    ];
    let root = VfsUri::root_file();
    let (root_name, root_kind) = if cfg!(windows) {
        ("This PC", PlaceKind::Computer)
    } else {
        ("File system", PlaceKind::Computer)
    };
    out.push((
        "Computer",
        vec![place(root_name.to_string(), root, root_kind, false)],
    ));
    let mut network = vec![
        place(
            "Network".into(),
            VfsUri::network(),
            PlaceKind::Computer,
            false,
        ),
        place(
            "Saved servers".into(),
            VfsUri::servers(),
            PlaceKind::Computer,
            false,
        ),
    ];
    network.extend(
        crate::servers::servers_of(&s.prefs.profiles)
            .into_iter()
            .map(|sv| place(sv.name, sv.uri, PlaceKind::Computer, false)),
    );
    out.push(("Network", network));
    out.retain(|(_, list)| !list.is_empty());
    out
}

/// Per-frame upkeep: once a sign-in finished, read the folder the picker was waiting on.
pub fn poll(s: &mut crate::Session) {
    let Some(folder) = s.picker.as_ref().map(|p| p.folder.clone()) else {
        return;
    };
    if matches!(s.dirs.state(&folder), crate::dircache::DirState::Absent)
        && s.backends.get_for(&folder).is_some()
        && let Err(err) = s.ensure_read(folder)
    {
        s.notice = Some(err.to_string());
    }
}

fn folder_target(s: &mut crate::Session, uri: &VfsUri) -> Result<(), FmError> {
    if !browsable(uri) {
        return Err(FmError::Unsupported("this location cannot be browsed"));
    }
    // An unconnected server opens its sign-in popup; the listing follows once connected.
    if matches!(uri.scheme(), Scheme::Sftp | Scheme::Smb) && s.backends.get_for(uri).is_none() {
        if !s.needs_sftp_login_to(uri, false) && !s.needs_login_to(uri, false) {
            return Err(FmError::Unsupported("this location cannot be browsed"));
        }
        return Ok(());
    }
    if browsable(uri) && !supported(uri) {
        s.ensure_read(uri.clone())?;
        return Ok(());
    }
    s.backends.require(uri)?;
    s.ensure_read(uri.clone())?;
    Ok(())
}

fn open() -> CommandSpec {
    CommandSpec {
        id: "picker.open",
        label: "Browse…",
        menu: &["File", "Browse…"],
        shortcut: None,
        params: r#"{purpose: string, kind?: "folder"|"file", title?: string, start?: string} (start: a folder or file; default: home)"#,
        enabled: |s| s.parked_pickers.len() < MAX_PARKED,
        run: |s, params| {
            let purpose = text(params, "purpose")?
                .filter(|p| !p.trim().is_empty())
                .ok_or_else(|| FmError::InvalidParams("purpose: required".into()))?
                .to_string();
            let kind = match text(params, "kind")? {
                None | Some("folder") => PickKind::Folder,
                Some("file") => PickKind::File,
                Some(_) => return Err(FmError::InvalidParams("kind: folder or file".into())),
            };
            let title = text(params, "title")?
                .unwrap_or_default()
                .trim()
                .to_string();
            // An unparsable or empty start falls back to home, so Browse always opens.
            let mut folder = text(params, "start")?
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .and_then(|t| VfsUri::parse(t).ok())
                .filter(supported)
                .unwrap_or_else(crate::workspace::home_location);
            if let Some(local) = folder.local_path().filter(|p| p.is_file())
                && let Some(parent) = local
                    .parent()
                    .and_then(|p| VfsUri::parse(&p.to_string_lossy()).ok())
            {
                folder = parent;
            }
            if let Some(local) = folder.local_path() {
                // A missing target folder: start at its nearest existing parent.
                let mut p = local.to_path_buf();
                while !p.is_dir() {
                    if !p.pop() {
                        break;
                    }
                }
                if let Ok(uri) = VfsUri::parse(&p.to_string_lossy()) {
                    folder = uri;
                }
            }
            folder_target(s, &folder).or_else(|_| {
                let home = crate::workspace::home_location();
                folder = home.clone();
                folder_target(s, &home)
            })?;
            if s.parked_pickers.len() >= MAX_PARKED {
                return Err(FmError::Unsupported("too many choosers are open"));
            }
            let nested = !s.parked_pickers.is_empty()
                || s.picker.is_some()
                || s.remote.sftp_prompt.is_some()
                || s.remote.login_prompt.is_some();
            if let Some(open) = s.picker.take() {
                s.parked_pickers.push(open);
            }
            s.picker = Some(Picker {
                purpose,
                kind,
                title,
                folder,
                selected: None,
                nested,
            });
            Ok(())
        },
    }
}

fn navigate() -> CommandSpec {
    CommandSpec {
        id: "picker.navigate",
        label: "Browse Folder",
        menu: &["File", "Browse Folder"],
        shortcut: None,
        params: "{uri: folder to list}",
        enabled: |s| s.picker.is_some(),
        run: |s, params| {
            let uri = object(params)?
                .and_then(|m| m.get("uri"))
                .ok_or_else(|| FmError::InvalidParams("uri: required".into()))
                .and_then(parse_uri)?;
            if s.picker.is_none() {
                return Err(FmError::Unsupported("no picker is open"));
            }
            folder_target(s, &uri)?;
            if let Some(p) = s.picker.as_mut() {
                p.folder = uri;
                p.selected = None;
            }
            Ok(())
        },
    }
}

fn select() -> CommandSpec {
    CommandSpec {
        id: "picker.select",
        label: "Select in Picker",
        menu: &["File", "Select in Picker"],
        shortcut: None,
        params: "{uri: the entry to select}",
        enabled: |s| s.picker.is_some(),
        run: |s, params| {
            let uri = object(params)?
                .and_then(|m| m.get("uri"))
                .ok_or_else(|| FmError::InvalidParams("uri: required".into()))
                .and_then(parse_uri)?;
            let picker = s
                .picker
                .as_mut()
                .ok_or(FmError::Unsupported("no picker is open"))?;
            picker.selected = Some(uri);
            Ok(())
        },
    }
}

fn accept() -> CommandSpec {
    CommandSpec {
        id: "picker.accept",
        label: "Choose",
        menu: &["File", "Choose"],
        shortcut: None,
        params: "none",
        enabled: |s| s.picker.is_some(),
        run: |s, params| {
            object(params)?;
            let picker = s
                .picker
                .clone()
                .ok_or(FmError::Unsupported("no picker is open"))?;
            let is_dir = |uri: &VfsUri| {
                s.dirs
                    .entries(&picker.folder)
                    .is_some_and(|e| e.iter().any(|e| e.uri == *uri && e.is_dir_like()))
            };
            let chosen = match picker.kind {
                PickKind::Folder => match picker.selected.as_ref() {
                    Some(sel) if is_dir(sel) => sel.clone(),
                    _ => picker.folder.clone(),
                },
                PickKind::File => match picker.selected.as_ref() {
                    Some(sel) if !is_dir(sel) => sel.clone(),
                    _ => return Err(FmError::InvalidParams("choose a file first".into())),
                },
            };
            if !choosable(&chosen) {
                return Err(FmError::InvalidParams(
                    "open a real folder first; this list cannot be chosen".into(),
                ));
            }
            let shown = crate::item_cmds::display_path(&chosen);
            match picker.purpose.as_str() {
                side @ ("sync.left" | "sync.right") => {
                    let field = side.trim_start_matches("sync.");
                    s.dispatch("sync.profile.set", &json!({ "patch": { field: shown } }))?;
                }
                _ => s.picked = Some((picker.purpose, shown)),
            }
            s.picker = s.parked_pickers.pop();
            Ok(())
        },
    }
}

/// `picker.mkdir {name}`: create a folder in the folder being listed.
fn mkdir() -> CommandSpec {
    CommandSpec {
        id: "picker.mkdir",
        label: "New Folder in Picker",
        menu: &["File", "New Folder in Picker"],
        shortcut: None,
        params: "{name: string}",
        enabled: |s| s.picker.as_ref().is_some_and(|p| choosable(&p.folder)),
        run: |s, params| {
            let name = text(params, "name")?
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| FmError::InvalidParams("name: required".into()))?
                .to_string();
            fm_ops::validate_name(&name)?;
            let parent = s
                .picker
                .as_ref()
                .map(|p| p.folder.clone())
                .filter(choosable)
                .ok_or(FmError::Unsupported("open a real folder first"))?;
            s.submit_op(crate::fileops::OpRequest::Mkdir { parent, name })
                .map(|_| ())
        },
    }
}

fn cancel() -> CommandSpec {
    CommandSpec {
        id: "picker.cancel",
        label: "Cancel Browse",
        menu: &["File", "Cancel Browse"],
        shortcut: None,
        params: "none",
        enabled: |s| s.picker.is_some(),
        run: |s, params| {
            object(params)?;
            s.picker = s.parked_pickers.pop();
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open(), navigate(), select(), accept(), mkdir(), cancel()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;

    #[test]
    fn accepting_a_folder_sets_the_sync_side() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        let start = tmp.path().to_string_lossy().to_string();
        s.dispatch(
            "picker.open",
            &json!({"purpose": "sync.right", "start": start}),
        )
        .expect("open picker");
        s.dispatch("picker.accept", &json!({})).expect("accept");
        assert!(s.picker.is_none());
        let st = s.sync.current_state().expect("state");
        let want = VfsUri::parse(&tmp.path().to_string_lossy()).expect("uri");
        assert_eq!(st.profile.right, want.to_string());
    }

    #[test]
    fn a_missing_start_folder_opens_at_the_nearest_parent() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        let start = tmp.path().join("not/yet").to_string_lossy().to_string();
        s.dispatch("picker.open", &json!({"purpose": "x", "start": start}))
            .expect("open");
        let p = s.picker.as_ref().expect("picker");
        assert_eq!(p.folder.local_path(), Some(tmp.path().to_path_buf()));
    }

    #[test]
    fn file_kind_needs_a_file_and_returns_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        std::fs::write(tmp.path().join("a.srt"), "x").expect("write");
        let mut s = Session::new().expect("session");
        s.dispatch(
            "picker.open",
            &json!({"purpose": "sub", "kind": "file", "start": tmp.path().to_string_lossy()}),
        )
        .expect("open");
        assert!(s.dispatch("picker.accept", &json!({})).is_err());
        for _ in 0..200 {
            s.poll();
            let folder = s.picker.as_ref().map(|p| p.folder.clone()).expect("picker");
            if matches!(s.dirs.state(&folder), crate::DirState::Ready { .. }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let file = VfsUri::parse(&tmp.path().join("a.srt").to_string_lossy()).expect("uri");
        s.dispatch("picker.select", &json!({"uri": file.to_string()}))
            .expect("select");
        s.dispatch("picker.accept", &json!({})).expect("accept");
        let (purpose, path) = s.picked.take().expect("picked");
        assert_eq!(purpose, "sub");
        assert!(path.ends_with("a.srt"));
    }

    #[test]
    fn places_offer_the_computer_and_network_and_gate_unconnected_servers() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        let names: Vec<_> = places(&s).iter().map(|(n, _)| *n).collect();
        assert!(names.contains(&"Computer") && names.contains(&"Network"));
        let start = tmp.path().to_string_lossy().to_string();
        s.dispatch("picker.open", &json!({"purpose": "x", "start": start}))
            .expect("open");
        s.dispatch("picker.navigate", &json!({"uri": "network:///"}))
            .expect("network is browsable");
        assert!(s.dispatch("picker.accept", &json!({})).is_err());
        s.dispatch("picker.navigate", &json!({"uri": "sftp://me@nas.example/"}))
            .expect("opens the sign-in popup");
        assert!(s.remote.sftp_prompt.is_some(), "sign-in popup opens");
    }

    #[test]
    fn mkdir_creates_a_folder_in_the_listed_folder() {
        let tmp = tempfile::tempdir().expect("tmp");
        let mut s = Session::new().expect("session");
        let start = tmp.path().to_string_lossy().to_string();
        s.dispatch("picker.open", &json!({"purpose": "x", "start": start}))
            .expect("open");
        for bad in [
            json!({}),
            json!({"name": ""}),
            json!({"name": "a/b"}),
            json!({"name": ".."}),
        ] {
            assert!(s.dispatch("picker.mkdir", &bad).is_err(), "{bad}");
        }
        s.dispatch("picker.mkdir", &json!({"name": "fresh"}))
            .expect("mkdir");
        for _ in 0..300 {
            s.poll();
            if tmp.path().join("fresh").is_dir() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(tmp.path().join("fresh").is_dir());
        s.dispatch("picker.navigate", &json!({"uri": "network:///"}))
            .expect("network");
        assert!(s.dispatch("picker.mkdir", &json!({"name": "x"})).is_err());
    }

    #[test]
    fn a_second_picker_parks_the_first_and_closing_it_restores_it() {
        let tmp = tempfile::tempdir().expect("tmp");
        let start = tmp.path().to_string_lossy().to_string();
        let mut s = Session::new().expect("session");
        s.dispatch("picker.open", &json!({"purpose": "first", "start": start}))
            .expect("first");
        s.dispatch("picker.open", &json!({"purpose": "key", "start": start}))
            .expect("second");
        assert_eq!(s.picker.as_ref().map(|p| p.purpose.as_str()), Some("key"));
        s.dispatch("picker.cancel", &json!({})).expect("cancel");
        assert_eq!(s.picker.as_ref().map(|p| p.purpose.as_str()), Some("first"));
        s.dispatch("picker.accept", &json!({})).expect("accept");
        assert!(s.picker.is_none() && s.parked_pickers.is_empty());
        assert_eq!(s.picked.as_ref().map(|(p, _)| p.as_str()), Some("first"));
    }

    #[test]
    fn bad_params_are_errors() {
        let mut s = Session::new().expect("session");
        for p in [
            json!({}),
            json!({"purpose": 5}),
            json!({"purpose": "x", "kind": "disk"}),
            json!({"purpose": "x", "start": 7}),
        ] {
            assert!(s.dispatch("picker.open", &p).is_err(), "{p}");
        }
        assert!(s.dispatch("picker.navigate", &json!({"uri": "/"})).is_err());
    }
}
