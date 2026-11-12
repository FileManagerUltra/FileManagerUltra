//! Drag & drop resolution (architecture §8). The UI only reports `(sources, destination,
//! modifiers)`; this module decides what happens, so the whole matrix is testable
//! without egui. [`resolve_drop`] is pure — the badge calls it every frame.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use fm_ops::ConflictPolicy;
use fm_vfs::{FmError, VfsUri};

use crate::backends::lane_for;
use crate::file_cmds::{object, parse_sources, parse_uri};
use crate::fileops::{MAX_SOURCES, OpRequest};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Where a drop came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropOrigin {
    Internal,
    External,
}

/// Modifier keys held at release (engine-side mirror; no egui types below L4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DropModifiers {
    pub ctrl: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropAction {
    Copy,
    Move,
}

/// What releasing the payload here would do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropResolution {
    Do(DropAction),
    /// Not a valid drop: the UI shows the `NoDrop` cursor.
    NoDrop(&'static str),
}

/// Decide the action for a drop. `same_device` is `None` while the engine has not yet
/// learned it; the safe default (copy) applies until it does.
pub fn resolve_drop(
    sources: &[VfsUri],
    dest: &VfsUri,
    mods: DropModifiers,
    origin: DropOrigin,
    same_device: Option<bool>,
) -> DropResolution {
    if sources.is_empty() {
        return DropResolution::NoDrop("nothing to drop");
    }
    if sources.len() > MAX_SOURCES {
        return DropResolution::NoDrop("too many items");
    }
    if mods.ctrl && mods.shift {
        return DropResolution::NoDrop("Ctrl+Shift is not a drop modifier");
    }
    // A source that is the destination or one of its ancestors would land inside itself.
    if sources.iter().any(|s| s.is_same_or_descendant_of(dest)) {
        return DropResolution::NoDrop("cannot drop an item into itself");
    }
    let already_there = sources.iter().all(|s| s.parent().as_ref() == Some(dest));
    if already_there && !mods.ctrl {
        return DropResolution::NoDrop("already in this folder");
    }
    if origin == DropOrigin::External {
        return DropResolution::Do(DropAction::Copy);
    }
    if mods.ctrl {
        return DropResolution::Do(DropAction::Copy);
    }
    if mods.shift {
        return DropResolution::Do(DropAction::Move);
    }
    match same_device {
        Some(true) => DropResolution::Do(DropAction::Move),
        _ => DropResolution::Do(DropAction::Copy),
    }
}

type DeviceKey = (VfsUri, VfsUri);

/// Same-device answers, learned off the UI thread (rule 4: no I/O per frame).
#[derive(Debug, Default)]
pub struct DeviceCache {
    known: HashMap<DeviceKey, bool>,
    probing: HashSet<DeviceKey>,
    inbox: Arc<Mutex<Vec<(DeviceKey, bool)>>>,
}

const DEVICE_CACHE_CAP: usize = 256;

impl Session {
    /// Cached answer to "are the sources on the destination's device?". Cross-backend is
    /// answered immediately; otherwise the first call enqueues a probe on the destination
    /// lane and returns `None`, and later calls see the result.
    pub fn same_device_hint(&mut self, sources: &[VfsUri], dest: &VfsUri) -> Option<bool> {
        let first = sources.first()?;
        if first.scheme() != dest.scheme() || first.authority() != dest.authority() {
            return Some(false);
        }
        let parent = first.parent()?;
        let key = (parent, dest.clone());
        if let Ok(mut inbox) = self.devices.inbox.lock() {
            for (k, v) in inbox.drain(..) {
                self.devices.probing.remove(&k);
                if self.devices.known.len() >= DEVICE_CACHE_CAP {
                    self.devices.known.clear();
                }
                self.devices.known.insert(k, v);
            }
        }
        if let Some(known) = self.devices.known.get(&key) {
            return Some(*known);
        }
        if self.devices.probing.contains(&key) {
            return None;
        }
        let backend = self.backends.require(dest).ok()?;
        let inbox = self.devices.inbox.clone();
        let probe_key = key.clone();
        self.devices.probing.insert(key);
        self.jobs.submit(lane_for(dest.scheme()), move |_ctx| {
            let same = backend.same_device(&probe_key.0, &probe_key.1);
            if let Ok(mut guard) = inbox.lock() {
                guard.push((probe_key, same));
            }
            Ok(())
        });
        None
    }

    /// What the badge shows while `sources` hover `dest`.
    pub fn preview_drop(
        &mut self,
        sources: &[VfsUri],
        dest: &VfsUri,
        mods: DropModifiers,
        origin: DropOrigin,
    ) -> DropResolution {
        let same = if origin == DropOrigin::Internal && !mods.ctrl && !mods.shift {
            self.same_device_hint(sources, dest)
        } else {
            None
        };
        resolve_drop(sources, dest, mods, origin, same)
    }
}

fn flag(map: Option<&crate::file_cmds::Obj>, key: &str) -> Result<bool, FmError> {
    match map.and_then(|m| m.get(key)) {
        None => Ok(false),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| FmError::InvalidParams(format!("{key} must be a boolean"))),
    }
}

fn drop_run(s: &mut Session, params: &Params) -> Result<(), FmError> {
    let map = object(params)?;
    let origin = match map.and_then(|m| m.get("origin")).map(Params::as_str) {
        None | Some(Some("internal")) => DropOrigin::Internal,
        Some(Some("external")) => DropOrigin::External,
        _ => {
            return Err(FmError::InvalidParams(
                "origin must be internal or external".to_string(),
            ));
        }
    };
    let dest = parse_uri(
        map.and_then(|m| m.get("dest"))
            .ok_or_else(|| FmError::InvalidParams("dest: uri is required".to_string()))?,
    )?;
    let raw = map
        .and_then(|m| m.get("sources"))
        .ok_or_else(|| FmError::InvalidParams("sources: [uri] is required".to_string()))?;
    let mods = DropModifiers {
        ctrl: flag(map, "ctrl")?,
        shift: flag(map, "shift")?,
    };

    // OS-provided paths are untrusted: a bad one is reported and the rest still import.
    let (sources, rejected) = if origin == DropOrigin::External {
        let list = raw.as_array().ok_or_else(|| {
            FmError::InvalidParams("sources must be an array of paths".to_string())
        })?;
        if list.len() > MAX_SOURCES {
            return Err(FmError::InvalidParams(format!(
                "too many sources (limit {MAX_SOURCES})"
            )));
        }
        let mut good = Vec::new();
        let mut bad = 0usize;
        for value in list {
            match parse_uri(value) {
                Ok(uri) => good.push(uri),
                Err(_) => bad += 1,
            }
        }
        (good, bad)
    } else {
        (parse_sources(raw)?, 0)
    };

    if !sources.is_empty() {
        let same = s.same_device_hint(&sources, &dest);
        match resolve_drop(&sources, &dest, mods, origin, same) {
            DropResolution::Do(action) => {
                let request = match action {
                    DropAction::Copy => OpRequest::Copy {
                        sources,
                        dest,
                        policy: ConflictPolicy::Ask,
                    },
                    DropAction::Move => OpRequest::Move {
                        sources,
                        dest,
                        policy: ConflictPolicy::Ask,
                    },
                };
                s.submit_op(request)?;
            }
            DropResolution::NoDrop(reason) => {
                return Err(FmError::InvalidParams(format!("drop rejected: {reason}")));
            }
        }
    }
    if rejected > 0 {
        return Err(FmError::InvalidParams(format!(
            "{rejected} dropped path(s) were rejected as invalid"
        )));
    }
    Ok(())
}

/// `dnd.drop` — a payload was released on a destination directory.
pub fn drop_command() -> CommandSpec {
    CommandSpec {
        id: "dnd.drop",
        label: "Drop",
        menu: &["File", "Drop"],
        shortcut: None,
        params: "sources: [uri], dest: uri, origin?: internal|external, ctrl?: bool, shift?: bool",
        enabled: |_s| true,
        run: drop_run,
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![drop_command()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    const NONE: DropModifiers = DropModifiers {
        ctrl: false,
        shift: false,
    };

    fn plain(same: Option<bool>) -> DropResolution {
        resolve_drop(&[u("/a/f.txt")], &u("/b"), NONE, DropOrigin::Internal, same)
    }

    #[test]
    fn modifier_matrix() {
        let src = [u("/a/f.txt")];
        let dest = u("/b");
        let r = |ctrl, shift, same| {
            resolve_drop(
                &src,
                &dest,
                DropModifiers { ctrl, shift },
                DropOrigin::Internal,
                same,
            )
        };
        assert_eq!(plain(Some(true)), DropResolution::Do(DropAction::Move));
        assert_eq!(plain(Some(false)), DropResolution::Do(DropAction::Copy));
        assert_eq!(plain(None), DropResolution::Do(DropAction::Copy));
        assert_eq!(
            r(true, false, Some(true)),
            DropResolution::Do(DropAction::Copy)
        );
        assert_eq!(
            r(false, true, Some(false)),
            DropResolution::Do(DropAction::Move)
        );
        assert!(matches!(
            r(true, true, Some(true)),
            DropResolution::NoDrop(_)
        ));
    }

    #[test]
    fn guards() {
        let d = DropOrigin::Internal;
        // Onto itself, onto a descendant of a source.
        assert!(matches!(
            resolve_drop(&[u("/a/dir")], &u("/a/dir"), NONE, d, Some(true)),
            DropResolution::NoDrop(_)
        ));
        assert!(matches!(
            resolve_drop(&[u("/a/dir")], &u("/a/dir/sub"), NONE, d, Some(true)),
            DropResolution::NoDrop(_)
        ));
        // Onto the folder the items already live in: nothing to do, except Ctrl (duplicate).
        assert!(matches!(
            resolve_drop(&[u("/a/f")], &u("/a"), NONE, d, Some(true)),
            DropResolution::NoDrop(_)
        ));
        // A sibling whose name merely shares a prefix is a valid target.
        assert_eq!(
            resolve_drop(&[u("/a/dir")], &u("/a/dir2"), NONE, d, Some(true)),
            DropResolution::Do(DropAction::Move)
        );
        assert!(matches!(
            resolve_drop(&[], &u("/a"), NONE, d, None),
            DropResolution::NoDrop(_)
        ));
    }

    #[test]
    fn external_drops_always_copy() {
        let mods = DropModifiers {
            ctrl: false,
            shift: true,
        };
        assert_eq!(
            resolve_drop(
                &[u("/home/x/f")],
                &u("/b"),
                mods,
                DropOrigin::External,
                Some(true)
            ),
            DropResolution::Do(DropAction::Copy)
        );
    }

    #[test]
    fn cross_backend_is_never_same_device() {
        let mut s = Session::new().expect("session");
        assert_eq!(
            s.same_device_hint(&[u("/a/f")], &u("sftp://host/b")),
            Some(false)
        );
    }

    #[test]
    #[cfg(unix)] // The local backend only knows device ids on Unix; elsewhere it says "different".
    fn same_device_is_learned_off_thread() {
        let mut s = Session::new().expect("session");
        let src = [u("/tmp/x")];
        let dest = u("/tmp");
        let mut got = s.same_device_hint(&src, &dest);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while got.is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
            got = s.same_device_hint(&src, &dest);
        }
        assert_eq!(got, Some(true));
    }
}
