//! `place.open`: open a bookmark or place. A remote place opens in a pane of its own kind
//! (Network for SMB, SFTP for SFTP), so the Files pane it was clicked in keeps its
//! location and folder tree.

use fm_layout::{Axis, PaneId, PaneKind};
use fm_vfs::{FmError, Scheme, VfsUri};
use serde_json::json;

use crate::file_cmds::{object, parse_uri};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// The pane kind a place on `uri` belongs in, when it is not a plain Files pane.
fn kind_for(uri: &VfsUri) -> Option<(PaneKind, &'static str)> {
    match uri.scheme() {
        Scheme::Smb => Some((PaneKind::Network, "network")),
        Scheme::Sftp => Some((PaneKind::Sftp, "sftp")),
        _ => None,
    }
}

fn params(s: &Session, p: &Params) -> Result<(VfsUri, PaneId), FmError> {
    let map = object(p)?.ok_or_else(|| FmError::InvalidParams("uri is required".into()))?;
    if map.keys().any(|key| key != "uri" && key != "pane") {
        return Err(FmError::InvalidParams("unknown parameter".into()));
    }
    let uri = map
        .get("uri")
        .ok_or_else(|| FmError::InvalidParams("uri is required".into()))
        .and_then(parse_uri)?;
    let pane = match map.get("pane") {
        None => s.layout.focused(),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| FmError::InvalidParams("pane must be a pane number".into()))?,
    };
    if s.layout.state(pane).is_none() {
        return Err(FmError::NotFound(format!("pane {pane}")));
    }
    Ok((uri, pane))
}

/// The pane `uri` opens in when opened from `from`: `from` itself for local places or when
/// it already is the right kind, else an existing pane of that kind, else a new one split
/// off `from`.
fn target(s: &mut Session, uri: &VfsUri, from: PaneId) -> Result<PaneId, FmError> {
    let Some((kind, name)) = kind_for(uri) else {
        return Ok(from);
    };
    let kind_of = |s: &Session, pane: PaneId| s.layout.state(pane).map(|st| st.kind);
    if kind_of(s, from) == Some(kind) {
        return Ok(from);
    }
    if let Some(existing) = s
        .layout
        .panes()
        .into_iter()
        .find(|pane| kind_of(s, *pane) == Some(kind))
    {
        return Ok(existing);
    }
    let pane = s.layout.split(from, Axis::Horizontal, 0.5)?;
    s.dispatch("pane.kind.set", &json!({"kind": name, "pane": pane}))?;
    Ok(pane)
}

pub fn open() -> CommandSpec {
    CommandSpec {
        id: "place.open",
        label: "Open Place",
        menu: &["Go", "Open Place"],
        shortcut: None,
        params: "{uri: string, pane?: number} — SMB/SFTP places open in a Network/SFTP pane (reused or split off), others in the pane",
        enabled: |_| true,
        run: |s, p| {
            let (uri, from) = params(s, p)?;
            let pane = target(s, &uri, from)?;
            s.layout.focus_pane(pane)?;
            s.dispatch("nav.open", &Params::String(uri.as_str()))
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![open()]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn kind(s: &Session, pane: PaneId) -> Option<PaneKind> {
        s.layout.state(pane).map(|st| st.kind)
    }

    fn files_session() -> (Session, PaneId, VfsUri) {
        let tmp = std::env::temp_dir();
        let here = VfsUri::parse(tmp.to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");
        let pane = s.layout.focused();
        if let Some(st) = s.layout.state_mut(pane) {
            *st = fm_layout::PaneState::new_rooted(here.clone(), VfsUri::root_file());
        }
        (s, pane, here)
    }

    #[test]
    fn an_smb_place_from_a_files_pane_opens_a_network_pane_and_signs_in_there() {
        let (mut s, files, here) = files_session();
        let tree_root = s.layout.state(files).expect("pane").tree.root.clone();
        s.dispatch(
            "place.open",
            &json!({"uri": "smb://192.168.0.50/", "pane": files}),
        )
        .expect("open");
        assert_eq!(s.layout.panes().len(), 2);
        let net = s.layout.focused();
        assert_ne!(net, files);
        assert_eq!(kind(&s, net), Some(PaneKind::Network));
        let prompt = s.remote.login_prompt.as_ref().expect("sign-in");
        assert_eq!(prompt.host, "192.168.0.50");
        assert!(prompt.navigate);
        // The Files pane is untouched.
        let st = s.layout.state(files).expect("pane");
        assert_eq!(
            (st.kind, &st.location, &st.tree.root),
            (PaneKind::Files, &here, &tree_root)
        );
    }

    #[test]
    fn an_existing_network_pane_is_reused_and_a_connected_share_opens_there() {
        let (mut s, files, here) = files_session();
        let share = VfsUri::parse("smb://nas/media").expect("uri");
        let fs = fm_testkit::MemFs::new().with_scheme(Scheme::Smb);
        fs.mkdirs(&share).expect("dir");
        s.backends.register_at(share.authority(), Arc::new(fs));
        let net = s.layout.split(files, Axis::Horizontal, 0.5).expect("split");
        s.dispatch("pane.kind.set", &json!({"kind": "network", "pane": net}))
            .expect("network pane");
        s.dispatch("place.open", &json!({"uri": share.as_str(), "pane": files}))
            .expect("open");
        assert_eq!(s.layout.panes().len(), 2, "no new pane");
        assert_eq!(s.layout.focused(), net);
        assert_eq!(s.layout.state(net).expect("pane").location, share);
        assert_eq!(s.layout.state(files).expect("pane").location, here);
    }

    #[test]
    fn an_sftp_place_opens_an_sftp_pane() {
        let (mut s, files, _) = files_session();
        s.dispatch(
            "place.open",
            &json!({"uri": "sftp://bob@host/srv", "pane": files}),
        )
        .expect("open");
        let pane = s.layout.focused();
        assert_ne!(pane, files);
        assert_eq!(kind(&s, pane), Some(PaneKind::Sftp));
        assert_eq!(kind(&s, files), Some(PaneKind::Files));
    }

    #[test]
    fn a_local_place_and_a_place_in_its_own_kind_open_in_the_pane() {
        let (mut s, files, _) = files_session();
        let home = crate::workspace::home_location();
        s.dispatch("place.open", &json!({"uri": home.as_str(), "pane": files}))
            .expect("open");
        assert_eq!(s.layout.panes().len(), 1);
        assert_eq!(s.layout.state(files).expect("pane").location, home);
        // From a Network pane an SMB place stays in that pane.
        s.dispatch("pane.kind.set", &json!({"kind": "network", "pane": files}))
            .expect("network");
        s.dispatch("place.open", &json!({"uri": "smb://nas/", "pane": files}))
            .expect("open");
        assert_eq!(s.layout.panes().len(), 1);
    }

    #[test]
    fn bad_params_are_errors_and_change_nothing() {
        let (mut s, _, _) = files_session();
        for p in [
            json!(null),
            json!({}),
            json!({"uri": 3}),
            json!({"uri": "smb://nas/", "pane": "x"}),
            json!({"uri": "smb://nas/", "pane": 999}),
            json!({"uri": "smb://nas/", "extra": 1}),
        ] {
            assert!(s.dispatch("place.open", &p).is_err(), "{p}");
        }
        assert_eq!(s.layout.panes().len(), 1);
    }
}
