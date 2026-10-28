//! Device actions validate against the cached list before starting background work.

use fm_vfs::{FmError, VfsUri};

use crate::Session;
use crate::devices::{Device, Removal};
use crate::registry::{CommandSpec, Params};

fn target(s: &Session, params: &Params, eject: bool) -> Result<Device, FmError> {
    let uri = params
        .as_object()
        .and_then(|p| p.get("uri"))
        .and_then(Params::as_str)
        .ok_or_else(|| FmError::InvalidParams("{uri: mounted volume URI} required".into()))?;
    let uri = VfsUri::parse(uri)?;
    let device = s
        .volumes
        .entries()
        .iter()
        .find(|d| d.mount == uri)
        .ok_or_else(|| {
            FmError::InvalidParams("URI is not a listed mounted volume; refresh Devices".into())
        })?;
    match &device.removal {
        Removal::Unsupported(reason) => return Err(FmError::backend("devices", reason)),
        Removal::Eject if !eject => {
            return Err(FmError::InvalidParams(
                "Use devices.eject for this drive".into(),
            ));
        }
        Removal::Dismount if eject => {
            return Err(FmError::InvalidParams(
                "Use devices.dismount for this volume".into(),
            ));
        }
        _ => {}
    }
    if s.volumes.removing() {
        return Err(FmError::backend(
            "devices",
            "Wait for the current removal to finish",
        ));
    }
    if s.volumes.scanning() {
        return Err(FmError::backend(
            "devices",
            "Devices is refreshing; try removal again when it finishes",
        ));
    }
    if s.fileops.uses_volume(&device.mount) {
        return Err(FmError::backend(
            "devices",
            "A file operation is using this volume. Wait for it to finish or cancel it before removal.",
        ));
    }
    Ok(device.clone())
}

fn remove(s: &mut Session, params: &Params, eject: bool) -> Result<(), FmError> {
    let device = target(s, params, eject)?;
    let home = s
        .standard_places
        .0
        .iter()
        .find(|p| p.kind == crate::places_cmds::PlaceKind::Home)
        .map(|p| p.uri.clone())
        .ok_or_else(|| FmError::backend("devices", "Cannot leave volume: Home is unavailable"))?;
    if device.mount.is_same_or_descendant_of(&home) {
        return Err(FmError::backend(
            "devices",
            "Home is on this volume; cannot remove it safely",
        ));
    }
    s.evacuate_volume(&device.mount, &home);
    s.dirs
        .invalidate_where(|uri| device.mount.is_same_or_descendant_of(uri));
    let worker_device = device.clone();
    let id = s.jobs.submit(crate::JobLane::Local, move |_ctx| {
        crate::devices::remove(&worker_device)
    });
    s.volumes.removal = Some((id, device));
    // A pending discovery result predating removal must not overwrite the next refresh.
    s.info = Some((
        "Removing volume; do not unplug it yet".into(),
        std::time::Instant::now(),
    ));
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "devices.refresh",
            label: "Refresh Devices",
            menu: &["Go", "Devices", "Refresh Devices"],
            shortcut: None,
            params: "none",
            enabled: |s| !s.volumes.scanning() && !s.volumes.removing(),
            run: |s, params| {
                if !params.is_null() && params.as_object().is_none_or(|m| !m.is_empty()) {
                    return Err(FmError::InvalidParams(
                        "devices.refresh takes no parameters".into(),
                    ));
                }
                s.volumes.refresh()
            },
        },
        CommandSpec {
            id: "devices.eject",
            label: "Eject Device",
            menu: &["Go", "Devices", "Eject Device"],
            shortcut: None,
            params: "{uri: string} - exact listed mount with an Eject capability",
            enabled: |s| {
                !s.volumes.removing()
                    && s.volumes
                        .entries()
                        .iter()
                        .any(|d| d.removal == Removal::Eject)
            },
            run: |s, params| remove(s, params, true),
        },
        CommandSpec {
            id: "devices.dismount",
            label: "Dismount Volume",
            menu: &["Go", "Devices", "Dismount Volume"],
            shortcut: None,
            params: "{uri: string} - exact listed mount with a supported Dismount capability",
            enabled: |s| {
                !s.volumes.removing()
                    && s.volumes
                        .entries()
                        .iter()
                        .any(|d| d.removal == Removal::Dismount)
            },
            run: |s, params| remove(s, params, false),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_ops::ConflictPolicy;
    use serde_json::json;

    fn fixture() -> Session {
        let mut s = Session::new().expect("session");
        s.volumes.replace(vec![Device {
            name: "USB".into(),
            mount: VfsUri::parse("/media/usb").expect("uri"),
            filesystem: "exfat".into(),
            total: None,
            free: None,
            identity: "42".into(),
            removal: Removal::Eject,
        }]);
        s
    }

    #[test]
    fn malformed_unlisted_and_wrong_capability_do_not_change_anything() {
        let mut s = fixture();
        let before = s.layout.state(1).expect("pane").location.clone();
        for id in ["devices.eject", "devices.dismount"] {
            for params in [
                Params::Null,
                json!({}),
                json!({"uri": 4}),
                json!({"uri": "/"}),
                json!({"uri": "/media/usb/child"}),
            ] {
                assert!(s.dispatch(id, &params).is_err(), "{id} {params}");
            }
        }
        assert!(target(&s, &json!({"uri": "/media/usb"}), false).is_err());
        assert!(!s.volumes.removing());
        assert_eq!(s.layout.state(1).expect("pane").location, before);
    }

    #[test]
    fn active_copy_source_as_well_as_destination_prevents_removal() {
        let mut s = fixture();
        let source = VfsUri::parse("/media/usb/file").expect("uri");
        // Even before the worker starts or finishes, the UI-side job table guards removal.
        s.submit_op(crate::OpRequest::Copy {
            sources: vec![source],
            dest: VfsUri::parse("/tmp").expect("uri"),
            policy: ConflictPolicy::Ask,
        })
        .expect("job");
        let error = target(&s, &json!({"uri": "/media/usb"}), true).expect_err("busy");
        assert!(error.to_string().contains("file operation"));
        assert!(!s.volumes.removing());
    }

    #[test]
    fn unknown_virtual_mount_is_browseable_but_never_ejected() {
        let mut s = fixture();
        let mut volume = s.volumes.entries()[0].clone();
        volume.removal = Removal::Unsupported("Dismount in VeraCrypt".into());
        s.volumes.replace(vec![volume]);
        let error = target(&s, &json!({"uri": "/media/usb"}), true).expect_err("unsupported");
        assert!(error.to_string().contains("VeraCrypt"));
        assert!(!s.volumes.removing());
    }

    #[test]
    fn removal_blocks_new_reads_navigation_and_transfers() {
        let mut s = fixture();
        let d = s.volumes.entries()[0].clone();
        s.volumes.removal = Some((999, d.clone()));
        assert!(
            s.dispatch("nav.open", &json!({"uri": d.mount.to_string()}))
                .is_err()
        );
        assert!(s.submit_read(d.mount.clone()).is_err());
        assert!(
            s.submit_op(crate::OpRequest::Mkdir {
                parent: d.mount,
                name: "test".into()
            })
            .is_err()
        );
    }

    #[test]
    fn evacuating_volume_moves_open_and_parked_workspaces_home() {
        let mut s = fixture();
        let d = s.volumes.entries()[0].clone();
        let home = VfsUri::parse("/home/example").expect("home");
        s.layout.state_mut(1).expect("pane").location = d.mount.clone();
        s.dispatch("workspace.new", &json!({"name": "Other"}))
            .expect("workspace");
        s.evacuate_volume(&d.mount, &home);
        s.dispatch("workspace.switch", &json!({"n": 1}))
            .expect("activate");
        assert_eq!(
            s.layout.state(s.layout.focused()).expect("pane").location,
            home
        );
    }
}
