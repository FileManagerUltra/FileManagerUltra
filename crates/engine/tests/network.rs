#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Network folder + sign-in popup, driven through commands only.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fm_discovery::{HostList, NetworkFs, Service};
use fm_engine::{DirState, Session};
use fm_layout::PaneKind;
use fm_vfs::{Scheme, VfsUri};
use serde_json::json;

fn pump(s: &mut Session, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        s.poll();
        if done(s) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn session() -> Session {
    let hosts = HostList::new();
    // Port 1 on loopback refuses at once, so a connect fails fast without a server.
    hosts.upsert("a", "nas", "10.9.9.9".parse().unwrap(), Service::Smb, 445);
    hosts.upsert("b", "pi", "10.9.9.8".parse().unwrap(), Service::Sftp, 22);
    let mut s = Session::new().unwrap();
    s.backends.register(Arc::new(NetworkFs::fixed(hosts)));
    s
}

fn smb_host() -> VfsUri {
    VfsUri::parse("smb://10.9.9.9/").unwrap()
}

#[test]
fn network_open_roots_the_tree_and_lists_hosts() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    let pane = s.layout.focused();
    let st = s.layout.state(pane).unwrap();
    assert_eq!(st.location, VfsUri::network());
    assert_eq!(st.tree.root, VfsUri::network());
    pump(&mut s, "network listing", |s| {
        s.dirs.state(&VfsUri::network()).is_ready()
    });
    let DirState::Ready { entries, .. } = s.dirs.state(&VfsUri::network()) else {
        panic!("not ready");
    };
    let names: Vec<_> = entries.iter().map(|e| e.name.clone()).collect();
    assert_eq!(names, ["nas (File Sharing)", "pi (Remote Login)"]);
}

#[test]
fn pane_kind_network_works_and_is_read_only() {
    let mut s = session();
    s.dispatch("pane.kind.set", &json!({"kind": "network"}))
        .unwrap();
    let pane = s.layout.focused();
    assert_ne!(s.layout.state(pane).unwrap().kind, PaneKind::Sftp);
    assert!(s.dispatch("nav.mkdir", &json!({"name": "x"})).is_err());
}

#[test]
fn opening_an_unconnected_host_asks_for_a_login_not_an_error() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    let uri = smb_host();
    s.ensure_read(uri.clone()).unwrap();
    let prompt = s.remote.login_prompt.clone().expect("popup");
    assert_eq!(prompt.host, "10.9.9.9");
    assert_eq!(prompt.domain, "WORKGROUP");
    assert!(prompt.error.is_none());
    assert!(s.notice.is_none());
    // Asking again while the popup is open does not stack another.
    s.ensure_read(uri).unwrap();
    assert!(s.remote.login_prompt.is_some());
}

#[test]
fn cancel_closes_the_popup_and_collapses_the_host() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    let uri = smb_host();
    let pane = s.layout.focused();
    s.layout.state_mut(pane).unwrap().tree.expand(uri.clone());
    s.ensure_read(uri.clone()).unwrap();
    s.dispatch("network.login.cancel", &json!(null)).unwrap();
    assert!(s.remote.login_prompt.is_none());
    assert!(!s.layout.state(pane).unwrap().tree.is_expanded(&uri));
}

#[test]
fn login_command_rejects_bad_params_and_keeps_the_popup() {
    let mut s = session();
    assert!(
        s.dispatch("network.login", &json!({"user": "a", "password": "b"}))
            .is_err()
    );
    s.ensure_read(smb_host()).unwrap();
    for bad in [
        json!(null),
        json!({}),
        json!({"user": "a"}),
        json!({"user": "", "password": "x"}),
        json!({"user": "a", "password": "x", "remember": "forever"}),
        json!({"user": 7, "password": "x"}),
        json!({"user": "a\nb", "password": "x"}),
    ] {
        assert!(s.dispatch("network.login", &bad).is_err(), "{bad}");
        assert!(s.remote.login_prompt.is_some(), "{bad}");
    }
}

#[test]
fn a_failed_connect_closes_the_popup_and_reports() {
    let mut s = session();
    let uri = VfsUri::parse("smb://127.0.0.1:1/").unwrap();
    s.ensure_read(uri.clone()).unwrap();
    s.dispatch(
        "network.login",
        &json!({"user": "alice", "password": "s3cret", "remember": "session"}),
    )
    .unwrap();
    assert!(s.remote.login_prompt.is_none());
    pump(&mut s, "failed connect", |s| {
        s.notice.is_some() || s.remote.login_prompt.is_some()
    });
    assert!(
        s.remote.login_prompt.is_none(),
        "unreachable is not a bad login"
    );
    let notice = s.notice.clone().unwrap_or_default();
    assert!(!notice.contains("s3cret"), "{notice}");
    assert!(
        !s.remote.remembers_login("127.0.0.1", Some(1)),
        "failures forget the login"
    );
}

#[test]
fn typed_smb_host_opens_the_popup() {
    let mut s = session();
    s.dispatch("nav.goto", &json!(null)).unwrap();
    s.dispatch("nav.goto.commit", &json!({"path": "smb://10.9.9.9/"}))
        .unwrap();
    let prompt = s.remote.login_prompt.clone().expect("popup");
    assert_eq!(prompt.host, "10.9.9.9");
    assert!(prompt.navigate, "the focused pane is on the host");
    s.dispatch("network.login.cancel", &json!(null)).unwrap();
    let pane = s.layout.focused();
    assert_ne!(s.layout.state(pane).unwrap().location.scheme(), Scheme::Smb);
}

#[test]
fn urls_with_a_user_open_the_popup_prefilled() {
    let mut s = session();
    s.ensure_read(VfsUri::parse("smb://alice@10.9.9.9/").unwrap())
        .ok();
    let prompt = s.remote.login_prompt.clone().expect("popup");
    assert_eq!(prompt.user, "alice");
    assert!(!prompt.manual);
}

#[test]
fn an_sftp_host_asks_first_then_opens_the_sftp_pane() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    let host = VfsUri::parse("sftp://10.9.9.8/").unwrap();
    s.ensure_read(host.clone()).unwrap();
    let pane = s.layout.focused();
    assert_ne!(s.layout.state(pane).unwrap().kind, PaneKind::Sftp);
    assert_eq!(s.remote.sftp_prompt, Some(host.clone()));
    s.dispatch("network.sftp.cancel", &json!(null)).unwrap();
    assert!(s.remote.sftp_prompt.is_none());
    assert_ne!(s.layout.state(pane).unwrap().kind, PaneKind::Sftp);
    s.ensure_read(host.clone()).unwrap();
    s.dispatch("network.sftp.open", &json!(null)).unwrap();
    let st = s.layout.state(pane).unwrap();
    assert_eq!(st.kind, PaneKind::Sftp);
    assert_eq!(st.location, host);
    assert!(s.remote.login_prompt.is_none());
    assert!(s.dispatch("network.sftp.open", &json!(null)).is_err());
}

#[test]
fn leaving_the_network_pane_restores_a_root_rooted_local_tree() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    s.dispatch("pane.kind.set", &json!({"kind": "files"}))
        .unwrap();
    let st = s.layout.state(s.layout.focused()).unwrap();
    assert_eq!(st.kind, PaneKind::Files);
    assert_eq!(st.tree.root, VfsUri::root_file());
    assert_eq!(st.location.scheme(), Scheme::File);
}

#[test]
fn network_folder_refreshes_when_hosts_appear_later() {
    let hosts = HostList::new();
    let mut s = Session::new().unwrap();
    s.backends
        .register(Arc::new(NetworkFs::fixed(hosts.clone())));
    s.dispatch("network.open", &json!(null)).unwrap();
    pump(&mut s, "first listing", |s| {
        s.dirs.state(&VfsUri::network()).is_ready()
    });
    hosts.upsert("a", "nas", "10.9.9.9".parse().unwrap(), Service::Smb, 445);
    pump(&mut s, "refresh", |s| {
        matches!(s.dirs.state(&VfsUri::network()),
            DirState::Ready { entries, .. } if entries.len() == 1)
    });
}
