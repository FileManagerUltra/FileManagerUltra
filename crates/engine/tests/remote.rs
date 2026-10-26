//! Engine-level SFTP behaviour against the in-process server (offline, unix only).
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::time::{Duration, Instant};

use fm_engine::Session;
use fm_engine::remote::{ConnState, KNOWN_HOSTS_FILE, KnownHosts};
use fm_fs_sftp::HostKeyInfo;
use fm_fs_sftp::testserver::TestServer;
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

fn session(config: &Path) -> Session {
    let mut s = Session::new().unwrap();
    s.remote.set_config_dir(config.to_path_buf());
    s
}

fn connect_args(server: &TestServer, password: &str) -> serde_json::Value {
    json!({
        "host": "127.0.0.1",
        "port": server.port(),
        "user": server.user,
        "password": password,
    })
}

fn authority(server: &TestServer) -> fm_vfs::Authority {
    VfsUri::parse(&format!(
        "sftp://{}@127.0.0.1:{}/",
        server.user,
        server.port()
    ))
    .unwrap()
    .authority()
    .clone()
}

fn is_ready(s: &Session, server: &TestServer) -> bool {
    s.remote.state(Scheme::Sftp, &authority(server)) == Some(&ConnState::Ready)
}

fn failed_message(s: &Session, server: &TestServer) -> Option<String> {
    match s.remote.state(Scheme::Sftp, &authority(server)) {
        Some(ConnState::Failed(m)) => Some(m.clone()),
        _ => None,
    }
}

fn start() -> (tempfile::TempDir, tempfile::TempDir, TestServer) {
    let root = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let server = TestServer::start(root.path(), "alice", "s3cret").unwrap();
    (root, config, server)
}

fn connect_and_trust(s: &mut Session, server: &TestServer) {
    s.dispatch("remote.connect", &connect_args(server, "s3cret"))
        .unwrap();
    pump(s, "host key prompt", |s| {
        s.remote.pending_host_key().is_some()
    });
    s.dispatch("remote.hostkey.answer", &json!({"accept": true}))
        .unwrap();
    pump(s, "connection", |s| is_ready(s, server));
}

#[test]
fn connect_asks_once_then_trusts_and_lists() {
    let (root, config, server) = start();
    std::fs::write(root.path().join("a.txt"), b"hi").unwrap();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);

    let pane = s.layout.state(s.layout.focused()).unwrap().location.clone();
    assert_eq!(pane.scheme(), Scheme::Sftp);
    let tree_root = s
        .layout
        .state(s.layout.focused())
        .unwrap()
        .tree
        .root
        .clone();
    assert_eq!(
        tree_root.scheme(),
        Scheme::Sftp,
        "tree must be the remote one"
    );
    pump(&mut s, "listing", |s| s.dirs.entries(&pane).is_some());
    let names: Vec<_> = s
        .dirs
        .entries(&pane)
        .unwrap()
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(names, ["a.txt"]);

    let file = config.path().join(KNOWN_HOSTS_FILE);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("SHA256:"));
    assert!(!text.contains("s3cret"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // A second session trusts the stored key without asking.
    let mut again = session(config.path());
    again
        .dispatch("remote.connect", &connect_args(&server, "s3cret"))
        .unwrap();
    pump(&mut again, "silent connect", |s| is_ready(s, &server));
}

#[test]
fn deleting_known_hosts_makes_a_running_app_ask_again() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    s.dispatch("remote.disconnect", &json!({})).unwrap();
    std::fs::remove_file(config.path().join(KNOWN_HOSTS_FILE)).unwrap();
    s.dispatch("remote.connect", &connect_args(&server, "s3cret"))
        .unwrap();
    pump(&mut s, "second host key prompt", |s| {
        s.remote.pending_host_key().is_some()
    });
}

#[test]
fn refused_host_key_fails_and_stores_nothing() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    s.dispatch("remote.connect", &connect_args(&server, "s3cret"))
        .unwrap();
    pump(&mut s, "prompt", |s| s.remote.pending_host_key().is_some());
    s.dispatch("remote.hostkey.answer", &json!({"accept": false}))
        .unwrap();
    pump(&mut s, "failure", |s| failed_message(s, &server).is_some());
    assert!(!config.path().join(KNOWN_HOSTS_FILE).exists());
}

#[test]
fn changed_host_key_is_a_hard_refusal() {
    let (_root, config, server) = start();
    let mut known = KnownHosts::default();
    known
        .add(&HostKeyInfo {
            host: "127.0.0.1".into(),
            port: server.port(),
            algorithm: "ssh-ed25519".into(),
            fingerprint: "SHA256:someoneelse".into(),
            openssh: "ssh-ed25519 AAAA".into(),
        })
        .unwrap();
    known.save(config.path()).unwrap();

    let mut s = session(config.path());
    s.dispatch("remote.connect", &connect_args(&server, "s3cret"))
        .unwrap();
    pump(&mut s, "failure", |s| failed_message(s, &server).is_some());
    assert!(
        s.remote.pending_host_key().is_none(),
        "no prompt for a changed key"
    );
    let msg = failed_message(&s, &server).unwrap();
    assert!(
        msg.contains("changed") && msg.contains("SHA256:someoneelse"),
        "{msg}"
    );
    assert!(s.remote.host_key_alert.is_some(), "UI card must be raised");
}

#[test]
fn wrong_password_fails_without_leaking_it() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    s.dispatch("remote.connect", &connect_args(&server, "canary-pw-123"))
        .unwrap();
    pump(&mut s, "prompt", |s| s.remote.pending_host_key().is_some());
    s.dispatch("remote.hostkey.answer", &json!({"accept": true}))
        .unwrap();
    pump(&mut s, "failure", |s| failed_message(s, &server).is_some());
    let msg = failed_message(&s, &server).unwrap();
    assert!(!msg.contains("canary-pw-123"));
    assert!(
        !s.remote
            .connection(Scheme::Sftp, &authority(&server))
            .unwrap()
            .can_reconnect()
    );
    assert!(
        !s.notice
            .clone()
            .unwrap_or_default()
            .contains("canary-pw-123")
    );
}

#[test]
fn lost_connection_disables_mutations_and_reconnects() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let pane = s.layout.state(s.layout.focused()).unwrap().location.clone();
    pump(&mut s, "listing", |s| s.dirs.entries(&pane).is_some());
    assert!(s.command_enabled("file.mkdir"));
    assert!(!s.command_enabled("remote.reconnect"));

    server.faults.kill_connections();
    pump(&mut s, "loss detected", |s| {
        s.refresh_remote_panes();
        failed_message(s, &server).is_some()
    });
    assert!(
        !s.command_enabled("file.mkdir"),
        "mutations stay off while stale"
    );
    assert!(s.command_enabled("remote.reconnect"));
    assert!(
        s.dirs.entries(&pane).is_some(),
        "stale listing stays visible"
    );

    s.dispatch("remote.reconnect", &json!(null)).unwrap();
    pump(&mut s, "reconnect", |s| is_ready(s, &server));
    assert!(s.command_enabled("file.mkdir"));
}

#[test]
fn background_poll_picks_up_remote_changes_without_blanking() {
    let (root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let pane = s.layout.state(s.layout.focused()).unwrap().location.clone();
    pump(&mut s, "listing", |s| s.dirs.entries(&pane).is_some());
    std::fs::write(root.path().join("new.txt"), b"x").unwrap();
    pump(&mut s, "poll refresh", |s| {
        s.refresh_remote_panes();
        assert!(s.dirs.entries(&pane).is_some(), "listing must not blank");
        s.dirs
            .entries(&pane)
            .is_some_and(|e| e.iter().any(|x| x.name == "new.txt"))
    });
}

#[test]
fn disconnect_returns_panes_to_local_disk() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    assert!(s.command_enabled("remote.disconnect"));
    let listed = s.layout.state(s.layout.focused()).unwrap().location.clone();
    let wait = listed.clone();
    pump(&mut s, "listing", move |s| s.dirs.entries(&wait).is_some());
    s.dispatch("remote.disconnect", &json!(null)).unwrap();
    assert!(s.remote.state(Scheme::Sftp, &authority(&server)).is_none());
    let st = s.layout.state(s.layout.focused()).unwrap();
    assert_eq!(st.location.scheme(), Scheme::File);
    assert_eq!(st.tree.root.scheme(), Scheme::File);
    assert!(s.dirs.entries(&listed).is_none(), "server listing dropped");
}

#[test]
fn disconnect_sends_an_sftp_pane_back_to_saved_servers() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let pane = s.layout.focused();
    s.layout.state_mut(pane).unwrap().kind = fm_layout::PaneKind::Sftp;
    s.dispatch("remote.disconnect", &json!(null)).unwrap();
    let st = s.layout.state(pane).unwrap();
    assert_eq!(st.location, fm_vfs::VfsUri::servers());
    assert_eq!(st.tree.root, fm_vfs::VfsUri::servers());
}

#[test]
fn free_space_never_blocks_and_is_cached() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let pane = s.layout.state(s.layout.focused()).unwrap().location.clone();
    assert!(s.free_space(&pane).is_none());
    pump(&mut s, "space job", |s| !s.remote.is_busy());
    let _ = s.free_space(&pane);
    assert!(
        !s.remote.is_busy(),
        "a fresh answer must not start another job"
    );
}

#[test]
fn bad_connect_params_are_errors() {
    let mut s = Session::new().unwrap();
    for params in [
        json!(null),
        json!({}),
        json!({"host": "h", "user": "u"}),
        json!({"host": "h", "user": "u", "password": 5}),
        json!({"host": "a b", "user": "u", "password": "p"}),
        json!({"host": "h", "user": "u", "password": "p", "port": 70000}),
        json!({"host": "h", "user": "u", "password": "p", "port": 0}),
        json!({"profile": "missing", "password": "p"}),
        json!({"host": "x".repeat(5000), "user": "u", "password": "p"}),
    ] {
        assert!(s.dispatch("remote.connect", &params).is_err(), "{params}");
    }
    assert!(
        s.dispatch("remote.hostkey.answer", &json!({"accept": true}))
            .is_err()
    );
    assert!(s.dispatch("remote.reconnect", &json!(null)).is_err());
    assert!(s.dispatch("remote.disconnect", &json!(null)).is_err());
}

#[test]
fn profiles_save_replace_delete_and_hold_no_secret() {
    let mut s = Session::new().unwrap();
    let args = json!({"name": "nas", "host": "nas.local", "user": "alice", "password": "ignored"});
    s.dispatch("remote.profile.save", &args).unwrap();
    s.dispatch(
        "remote.profile.save",
        &json!({"name": "nas", "host": "nas2.local", "user": "alice", "port": 2222}),
    )
    .unwrap();
    assert_eq!(s.prefs.profiles.len(), 1);
    assert_eq!(s.prefs.profiles[0].host, "nas2.local");
    let text = toml::to_string(&s.prefs).unwrap();
    assert!(!text.contains("ignored") && !text.contains("password"));
    assert!(
        s.dispatch("remote.profile.save", &json!({"name": "x"}))
            .is_err()
    );
    s.dispatch("remote.profile.delete", &json!({"name": "nas"}))
        .unwrap();
    assert!(
        s.dispatch("remote.profile.delete", &json!({"name": "nas"}))
            .is_err()
    );
    for i in 0..50 {
        s.dispatch(
            "remote.profile.save",
            &json!({"name": format!("p{i}"), "host": "h", "user": "u"}),
        )
        .unwrap();
    }
    assert!(
        s.dispatch(
            "remote.profile.save",
            &json!({"name": "one-too-many", "host": "h", "user": "u"})
        )
        .is_err()
    );
}

#[test]
fn known_hosts_survive_corruption_and_caps() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(KNOWN_HOSTS_FILE), "{ not json").unwrap();
    assert!(KnownHosts::load(dir.path()).is_empty());
    let huge = "x".repeat(10_000);
    let doc = json!({"hosts": [
        {"host": "a:22", "algorithm": "k", "fingerprint": "SHA256:a", "openssh": "k"},
        {"host": "b:22"},
        {"host": huge, "algorithm": "k", "fingerprint": "f", "openssh": "k"},
    ]});
    std::fs::write(dir.path().join(KNOWN_HOSTS_FILE), doc.to_string()).unwrap();
    assert_eq!(KnownHosts::load(dir.path()).len(), 1);

    let mut known = KnownHosts::default();
    for i in 0..1000 {
        let info = HostKeyInfo {
            host: format!("h{i}"),
            port: 22,
            algorithm: "k".into(),
            fingerprint: format!("f{i}"),
            openssh: "k".into(),
        };
        known.add(&info).unwrap();
    }
    let extra = HostKeyInfo {
        host: "more".into(),
        port: 22,
        algorithm: "k".into(),
        fingerprint: "f".into(),
        openssh: "k".into(),
    };
    assert!(known.add(&extra).is_err());
}

mod transfers {
    use super::*;
    use fm_engine::OpState;
    use std::sync::atomic::Ordering;

    fn remote_uri(server: &TestServer, path: &str) -> String {
        format!("sftp://{}@127.0.0.1:{}{path}", server.user, server.port())
    }

    fn run_op(s: &mut Session, id: &str, params: serde_json::Value) -> OpState {
        s.dispatch(id, &params).unwrap();
        let job = s.fileops.jobs().last().unwrap().id;
        pump(s, "job", |s| {
            s.fileops.get(job).is_some_and(|j| !j.state.is_running())
        });
        s.fileops.get(job).unwrap().state.clone()
    }

    fn no_partials(root: &Path) {
        for entry in std::fs::read_dir(root).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(!name.starts_with(".fm"), "temp file left behind: {name}");
        }
    }

    fn payload() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn local_to_sftp_and_back() {
        let (root, config, server) = start();
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("big.bin"), payload()).unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server);

        let state = run_op(
            &mut s,
            "file.copy",
            json!({"sources": [local.path().join("big.bin")], "dest": remote_uri(&server, "/")}),
        );
        assert_eq!(state, OpState::Finished);
        assert_eq!(
            std::fs::read(root.path().join("big.bin")).unwrap(),
            payload()
        );
        no_partials(root.path());

        let back = tempfile::tempdir().unwrap();
        let state = run_op(
            &mut s,
            "file.copy",
            json!({"sources": [remote_uri(&server, "/big.bin")], "dest": back.path()}),
        );
        assert_eq!(state, OpState::Finished);
        assert_eq!(
            std::fs::read(back.path().join("big.bin")).unwrap(),
            payload()
        );
        no_partials(back.path());
    }

    #[test]
    fn sftp_to_sftp_between_servers() {
        let (root_a, config, server_a) = start();
        let root_b = tempfile::tempdir().unwrap();
        let server_b = TestServer::start(root_b.path(), "alice", "s3cret").unwrap();
        std::fs::create_dir(root_a.path().join("tree")).unwrap();
        std::fs::write(root_a.path().join("tree/one.txt"), b"one").unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server_a);
        connect_and_trust(&mut s, &server_b);

        let state = run_op(
            &mut s,
            "file.copy",
            json!({"sources": [remote_uri(&server_a, "/tree")], "dest": remote_uri(&server_b, "/")}),
        );
        assert_eq!(state, OpState::Finished);
        assert_eq!(
            std::fs::read(root_b.path().join("tree/one.txt")).unwrap(),
            b"one"
        );
        assert!(
            root_a.path().join("tree/one.txt").exists(),
            "copy keeps the source"
        );
    }

    #[test]
    fn cross_backend_move_removes_source_after_copy() {
        let (root, config, server) = start();
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("m.txt"), b"moved").unwrap();
        std::fs::write(root.path().join("r.txt"), b"remote").unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server);

        let state = run_op(
            &mut s,
            "file.move",
            json!({"sources": [local.path().join("m.txt")], "dest": remote_uri(&server, "/")}),
        );
        assert_eq!(state, OpState::Finished);
        assert!(!local.path().join("m.txt").exists());
        assert_eq!(std::fs::read(root.path().join("m.txt")).unwrap(), b"moved");

        let state = run_op(
            &mut s,
            "file.move",
            json!({"sources": [remote_uri(&server, "/r.txt")], "dest": local.path()}),
        );
        assert_eq!(state, OpState::Finished);
        assert!(!root.path().join("r.txt").exists());
        assert_eq!(
            std::fs::read(local.path().join("r.txt")).unwrap(),
            b"remote"
        );
    }

    #[test]
    fn same_server_move_renames_in_place() {
        let (root, config, server) = start();
        std::fs::create_dir(root.path().join("sub")).unwrap();
        std::fs::write(root.path().join("f.txt"), b"x").unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server);
        let state = run_op(
            &mut s,
            "file.move",
            json!({"sources": [remote_uri(&server, "/f.txt")], "dest": remote_uri(&server, "/sub")}),
        );
        assert_eq!(state, OpState::Finished);
        assert!(!root.path().join("f.txt").exists());
        assert_eq!(std::fs::read(root.path().join("sub/f.txt")).unwrap(), b"x");
    }

    #[test]
    fn cancel_mid_transfer_leaves_no_partial_file() {
        let (root, config, server) = start();
        std::fs::write(root.path().join("slow.bin"), payload()).unwrap();
        server.faults.read_delay_ms.store(40, Ordering::Relaxed);
        let local = tempfile::tempdir().unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server);

        s.dispatch(
            "file.copy",
            &json!({"sources": [remote_uri(&server, "/slow.bin")], "dest": local.path()}),
        )
        .unwrap();
        let job = s.fileops.jobs().last().unwrap().id;
        pump(&mut s, "transfer to start", |s| {
            s.fileops
                .get(job)
                .is_some_and(|j| j.progress.bytes.done > 0)
        });
        s.cancel_op(job).unwrap();
        pump(&mut s, "cancel", |s| {
            s.fileops.get(job).is_some_and(|j| !j.state.is_running())
        });
        assert_eq!(s.fileops.get(job).unwrap().state, OpState::Cancelled);
        assert!(!local.path().join("slow.bin").exists());
        no_partials(local.path());
    }

    #[test]
    fn failed_copy_keeps_the_source() {
        let (root, config, server) = start();
        let local = tempfile::tempdir().unwrap();
        std::fs::write(local.path().join("keep.txt"), b"precious").unwrap();
        let mut s = session(config.path());
        connect_and_trust(&mut s, &server);
        server.faults.deny_open.store(true, Ordering::Relaxed);

        let _ = run_op(
            &mut s,
            "file.move",
            json!({"sources": [local.path().join("keep.txt")], "dest": remote_uri(&server, "/")}),
        );
        assert_eq!(
            std::fs::read(local.path().join("keep.txt")).unwrap(),
            b"precious"
        );
        assert!(!root.path().join("keep.txt").exists());
        no_partials(root.path());
    }
}

#[test]
fn connect_command_opens_the_popup() {
    let mut s = Session::new().unwrap();
    s.dispatch("remote.connect.dialog", &json!(null)).unwrap();
    let prompt = s.remote.sftp_prompt.clone().expect("popup");
    assert!(prompt.navigate && prompt.landing.is_none() && !prompt.editing);
    s.dispatch("remote.connect.cancel", &json!(null)).unwrap();
    assert!(s.remote.sftp_prompt.is_none());
}

#[test]
fn opening_a_remote_folder_keeps_the_pane_on_the_server() {
    let (root, config, server) = start();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let here = s.layout.state(s.layout.focused()).unwrap().location.clone();
    let sub = here.join("sub").unwrap();
    s.dispatch("file.open", &json!({ "uri": sub.to_string() }))
        .unwrap();
    let pane = s.layout.state(s.layout.focused()).unwrap();
    assert_eq!(pane.location, sub);
    assert_eq!(pane.location.scheme(), Scheme::Sftp);
}

#[test]
fn remote_delete_is_permanent_only() {
    let (root, config, server) = start();
    std::fs::write(root.path().join("gone.txt"), b"x").unwrap();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    let here = s.layout.state(s.layout.focused()).unwrap().location.clone();
    let file = here.join("gone.txt").unwrap();
    let sources = json!([file.to_string()]);

    let err = s
        .dispatch("file.delete", &json!({ "sources": sources }))
        .unwrap_err();
    assert!(err.to_string().contains("no trash"), "{err}");
    assert!(root.path().join("gone.txt").exists());

    s.dispatch(
        "file.delete_permanent",
        &json!({ "sources": sources, "confirm": true }),
    )
    .unwrap();
    pump(&mut s, "delete", |_| !root.path().join("gone.txt").exists());
}

#[test]
fn typed_sftp_address_opens_the_popup_for_that_server() {
    let mut s = Session::new().unwrap();
    s.dispatch("nav.goto", &json!(null)).unwrap();
    s.dispatch(
        "nav.goto.commit",
        &json!({"path": "sftp://bob@example.test:2222/srv/data"}),
    )
    .unwrap();
    let prompt = s.remote.sftp_prompt.clone().expect("popup");
    assert_eq!(
        (prompt.host.as_str(), prompt.port),
        ("example.test", Some(2222))
    );
    assert_eq!(prompt.user, "bob");
    assert!(s.path_edit.is_none());
}

#[test]
fn forgetting_a_host_key_persists_and_asks_again() {
    let (_root, config, server) = start();
    let mut s = session(config.path());
    connect_and_trust(&mut s, &server);
    s.dispatch("remote.disconnect", &json!({})).unwrap();
    let id = s.remote.known_hosts().entries()[0].host.clone();
    assert!(s.dispatch("remote.hostkey.forget", &json!({})).is_err());
    assert!(
        s.dispatch("remote.hostkey.forget", &json!({"host": "nope:1"}))
            .is_err()
    );
    s.dispatch("remote.hostkey.forget", &json!({"host": id}))
        .unwrap();
    assert!(s.remote.known_hosts().is_empty());
    let text = std::fs::read_to_string(config.path().join(KNOWN_HOSTS_FILE)).unwrap();
    assert!(!text.contains(&id));
    s.dispatch("remote.connect", &connect_args(&server, "s3cret"))
        .unwrap();
    pump(&mut s, "prompt again", |s| {
        s.remote.pending_host_key().is_some()
    });
}

#[test]
fn smb_connect_rejects_bad_params_without_connecting() {
    let mut s = Session::new().unwrap();
    for params in [
        json!({}),
        json!({"host": "nas", "user": "a", "password": "p"}),
        json!({"host": "nas", "share": "s", "user": "a"}),
        json!({"host": "nas", "share": "s", "user": "a", "password": "p", "port": 0}),
        json!({"host": "nas", "share": "s", "user": "a", "password": "p", "require_encryption": "yes"}),
        json!({"host": "bad host", "share": "s", "user": "a", "password": "p"}),
        json!("nope"),
    ] {
        assert!(
            s.dispatch("remote.connect.smb", &params).is_err(),
            "{params}"
        );
    }
    assert!(!s.remote.is_busy());
}

#[test]
fn network_connect_dialog_opens_a_manual_prompt_and_validates() {
    let mut s = Session::new().unwrap();
    s.dispatch("network.connect.dialog", &json!(null))
        .expect("dialog");
    assert!(s.remote.login_prompt.as_ref().is_some_and(|p| p.manual));
    for params in [
        json!({"user": "a", "password": "p"}),
        json!({"user": "a", "password": "p", "host": "bad host"}),
        json!({"user": "a", "password": "p", "host": "h", "port": 0}),
        json!({"user": "a", "password": "p", "host": "h", "port": "x"}),
    ] {
        assert!(s.dispatch("network.login", &params).is_err(), "{params}");
        assert!(s.remote.login_prompt.is_some(), "{params}");
    }
    s.dispatch("network.login.cancel", &json!(null))
        .expect("cancel");
    assert!(s.remote.login_prompt.is_none());
}

#[test]
fn sftp_and_smb_connections_to_one_host_do_not_collide() {
    let mut s = Session::new().unwrap();
    let uri = VfsUri::parse("smb://alice@nas.local/share").expect("uri");
    s.remote
        .set_state_for_tests(Scheme::Smb, uri.authority(), ConnState::Ready);
    assert!(s.remote.state(Scheme::Smb, uri.authority()).is_some());
    assert!(s.remote.state(Scheme::Sftp, uri.authority()).is_none());
    assert!(s.disconnect(Scheme::Sftp, uri.authority()).is_err());
    s.disconnect(Scheme::Smb, uri.authority())
        .expect("disconnect");
}
