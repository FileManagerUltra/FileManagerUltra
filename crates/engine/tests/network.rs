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
fn an_sftp_address_asks_in_the_popup_and_cancel_goes_back() {
    let mut s = session();
    s.dispatch("network.open", &json!(null)).unwrap();
    let host = VfsUri::parse("sftp://10.9.9.8/").unwrap();
    s.ensure_read(host.clone()).unwrap();
    let pane = s.layout.focused();
    assert_ne!(s.layout.state(pane).unwrap().kind, PaneKind::Sftp);
    let prompt = s.remote.sftp_prompt.clone().expect("popup");
    assert_eq!(prompt.host, "10.9.9.8");
    assert_eq!(prompt.landing, Some(host));
    s.dispatch("remote.connect.cancel", &json!(null)).unwrap();
    assert!(s.remote.sftp_prompt.is_none());
    assert_eq!(s.layout.state(pane).unwrap().location, VfsUri::network());
}

fn saved_server(s: &mut Session) {
    s.dispatch(
        "remote.profile.save",
        &json!({"name": "nas", "host": "10.9.9.7", "user": "bob", "port": 2222, "start_dir": "/data"}),
    )
    .unwrap();
}

#[test]
fn an_sftp_pane_without_a_connection_lists_the_saved_servers() {
    let mut s = session();
    saved_server(&mut s);
    let pane = s.layout.focused();
    s.dispatch("pane.kind.set", &json!({"kind": "sftp"}))
        .unwrap();
    let st = s.layout.state(pane).unwrap();
    assert_eq!(st.kind, PaneKind::Sftp);
    assert_eq!(st.location, VfsUri::servers());
    assert_eq!(st.tree.root, VfsUri::servers());
    for _ in 0..200 {
        s.poll();
        if matches!(s.dirs.state(&VfsUri::servers()), DirState::Ready { .. }) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let entries = s.dirs.entries(&VfsUri::servers()).expect("listed");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "nas");
    assert_eq!(entries[0].uri.to_string(), "sftp://bob@10.9.9.7:2222/");
}

#[test]
fn opening_a_saved_server_prefills_the_popup_and_edit_never_carries_a_password() {
    let mut s = session();
    saved_server(&mut s);
    s.ensure_read(VfsUri::parse("sftp://bob@10.9.9.7:2222/").unwrap())
        .ok();
    let p = s.remote.sftp_prompt.clone().expect("popup");
    assert_eq!(
        (p.user.as_str(), p.start_dir.as_str(), p.save_as.as_str()),
        ("bob", "/data", "nas")
    );
    s.dispatch("remote.connect.cancel", &json!(null)).unwrap();
    s.dispatch("servers.edit", &json!({"name": "nas"})).unwrap();
    let p = s.remote.sftp_prompt.clone().expect("popup");
    assert!(p.editing && p.landing.is_none());
    assert_eq!(p.port, Some(2222));
    assert!(
        s.dispatch("servers.edit", &json!({"name": "nope"}))
            .is_err()
    );
    assert!(s.dispatch("servers.edit", &json!({})).is_err());
    assert!(s.dispatch("servers.edit", &json!(null)).is_err());
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

#[test]
fn a_failed_sftp_connect_keeps_the_popup_open_with_the_error() {
    let mut s = session();
    s.ensure_read(VfsUri::parse("sftp://127.0.0.1:1/").unwrap())
        .unwrap();
    assert!(s.remote.sftp_prompt.is_some());
    s.dispatch(
        "remote.connect",
        &json!({"host": "127.0.0.1", "port": 1, "user": "alice", "password": "s3cret"}),
    )
    .unwrap();
    pump(&mut s, "failed connect", |s| {
        s.remote
            .sftp_prompt
            .as_ref()
            .is_some_and(|p| p.error.is_some())
    });
    let error = s.remote.sftp_prompt.clone().and_then(|p| p.error).unwrap();
    assert!(!error.contains("s3cret"), "{error}");
    assert!(s.notice.is_none(), "the error lives in the popup");
}

#[test]
fn a_failed_remembered_sftp_connect_opens_a_prefilled_retry_dialog() {
    let mut s = session();
    s.dispatch(
        "remote.profile.save",
        &json!({
            "name": "local nas", "host": "127.0.0.1", "port": 1,
            "user": "alice", "start_dir": "/data"
        }),
    )
    .unwrap();
    s.dispatch("remote.connect.dialog", &json!(null)).unwrap();
    s.dispatch(
        "remote.connect",
        &json!({
            "host": "127.0.0.1", "port": 1, "user": "alice",
            "password": "private-password", "remember": true
        }),
    )
    .unwrap();
    pump(&mut s, "first failure", |s| !s.remote.is_connecting());
    s.dispatch("remote.connect.cancel", &json!(null)).unwrap();
    s.notice = None;
    let uri = VfsUri::parse("sftp://alice@127.0.0.1:1/").unwrap();
    assert!(s.dispatch_or_notice("nav.open", &json!(uri.to_string())));
    assert!(
        s.notice.is_none(),
        "opening a disconnected server is not unsupported"
    );
    assert!(
        s.remote.sftp_prompt.is_none(),
        "remembered connect starts silently"
    );
    pump(&mut s, "retry dialog", |s| s.remote.sftp_prompt.is_some());
    let prompt = s.remote.sftp_prompt.as_ref().unwrap();
    assert_eq!(prompt.host, "127.0.0.1");
    assert_eq!(prompt.port, Some(1));
    assert_eq!(prompt.user, "alice");
    assert_eq!(prompt.start_dir, "/data");
    assert_eq!(prompt.save_as, "local nas");
    assert_eq!(prompt.landing.as_ref(), Some(&uri));
    assert!(prompt.navigate);
    let error = prompt.error.as_ref().expect("error in dialog");
    assert!(error.contains("network or VPN"), "{error}");
    assert!(!error.contains("private-password"));
    assert!(
        s.notice.is_none(),
        "connection failure is not status-bar-only"
    );
}

#[test]
fn opening_a_saved_server_row_opens_login_without_a_backend_error() {
    let mut s = session();
    saved_server(&mut s);
    s.dispatch("pane.kind.set", &json!({"kind": "sftp"}))
        .unwrap();
    let before = s.layout.state(s.layout.focused()).unwrap().location.clone();
    let uri = VfsUri::parse("sftp://bob@10.9.9.7:2222/").unwrap();
    assert!(s.dispatch_or_notice("nav.open", &json!(uri.to_string())));
    assert!(s.notice.is_none());
    let prompt = s.remote.sftp_prompt.as_ref().expect("login dialog");
    assert_eq!(prompt.save_as, "nas");
    assert_eq!(prompt.start_dir, "/data");
    assert!(prompt.navigate);
    assert_eq!(s.layout.state(s.layout.focused()).unwrap().location, before);
}

#[test]
fn cancelling_an_explicit_sftp_connect_does_not_reopen_the_dialog_on_failure() {
    let mut s = session();
    s.dispatch("remote.connect.dialog", &json!(null)).unwrap();
    s.dispatch(
        "remote.connect",
        &json!({
            "host": "127.0.0.1", "port": 1, "user": "alice", "password": "secret"
        }),
    )
    .unwrap();
    s.dispatch("remote.connect.cancel", &json!(null)).unwrap();
    pump(&mut s, "cancelled dialog failure", |s| {
        !s.remote.is_connecting()
    });
    assert!(s.remote.sftp_prompt.is_none());
    assert!(
        s.notice.is_some(),
        "cancelled attempts still report failure"
    );
}
