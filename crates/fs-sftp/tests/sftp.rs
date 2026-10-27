//! Round trips against the in-process SFTP server (offline, unix only).
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use fm_fs_sftp::testserver::{TestServer, client_key};
use fm_fs_sftp::{
    AuthMethod, ConnectParams, HostKeyInfo, HostKeyVerdict, HostKeyVerifier, SecretString, SftpFs,
    connect, is_transport_error,
};
use fm_vfs::{EntryKind, FmError, Vfs, VfsUri, WriteOpts};

struct TrustAll;
impl HostKeyVerifier for TrustAll {
    fn verify(&self, _: &HostKeyInfo) -> HostKeyVerdict {
        HostKeyVerdict::Trust
    }
}

fn params(server: &TestServer, password: &str) -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port: server.port(),
        user: server.user.clone(),
        auth: AuthMethod::Password(SecretString::from(password.to_string())),
        start_dir: None,
    }
}

fn setup() -> (tempfile::TempDir, TestServer, SftpFs) {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "s3cret").unwrap();
    let fs = connect(&params(&server, "s3cret"), Arc::new(TrustAll)).unwrap();
    (dir, server, fs)
}

fn uri(fs: &SftpFs, path: &str) -> VfsUri {
    VfsUri::parse(&format!("sftp://{}{path}", fs.authority())).unwrap()
}

#[test]
fn round_trip() {
    let (_dir, _server, fs) = setup();
    assert_eq!(fs.start_uri().unwrap().path_str(), "/");
    let file = uri(&fs, "/a.txt");
    {
        let mut w = fs.open_write(&file, WriteOpts::default()).unwrap();
        w.write_all(b"hello world").unwrap();
        w.flush().unwrap();
    }
    let list = fs.read_dir(&uri(&fs, "/")).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "a.txt");
    assert_eq!(list[0].size, Some(11));
    assert!(list[0].modified.is_some());
    assert_eq!(fs.stat(&file).unwrap().kind, EntryKind::File);

    let mut got = String::new();
    fs.open_read(&file)
        .unwrap()
        .read_to_string(&mut got)
        .unwrap();
    assert_eq!(got, "hello world");

    let moved = uri(&fs, "/b.txt");
    fs.rename(&file, &moved).unwrap();
    assert!(matches!(fs.stat(&file), Err(FmError::NotFound(_))));
    fs.remove(&moved).unwrap();
    assert!(fs.read_dir(&uri(&fs, "/")).unwrap().is_empty());
}

#[test]
fn directories_symlinks_and_conflicts() {
    let (dir, _server, fs) = setup();
    let sub = uri(&fs, "/sub");
    fs.create_dir(&sub).unwrap();
    assert!(matches!(
        fs.create_dir(&sub),
        Err(FmError::AlreadyExists(_))
    ));
    assert!(fs.stat(&sub).unwrap().is_dir());

    let link = uri(&fs, "/link");
    fs.create_symlink("sub", &link).unwrap();
    let e = fs.stat(&link).unwrap();
    assert_eq!(
        e.kind,
        EntryKind::Symlink {
            target: "sub".into()
        }
    );
    assert!(e.link_to_dir);
    assert_eq!(
        std::fs::read_link(dir.path().join("link"))
            .unwrap()
            .to_str(),
        Some("sub")
    );

    std::fs::write(dir.path().join("sub/x"), b"1").unwrap();
    assert!(matches!(fs.remove(&sub), Err(FmError::NotEmpty(_))));
    let target = uri(&fs, "/sub2");
    fs.create_dir(&target).unwrap();
    assert!(matches!(
        fs.rename(&sub, &target),
        Err(FmError::AlreadyExists(_))
    ));
    fs.remove(&link).unwrap();
}

#[test]
fn resume_offset_yields_the_tail() {
    let (dir, _server, fs) = setup();
    std::fs::write(dir.path().join("t"), b"0123456789").unwrap();
    let mut got = String::new();
    fs.open_read_at(&uri(&fs, "/t"), 5)
        .unwrap()
        .read_to_string(&mut got)
        .unwrap();
    assert_eq!(got, "56789");
}

#[test]
fn large_file_streams_intact() {
    let (_dir, _server, fs) = setup();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let file = uri(&fs, "/big");
    let mut w = fs.open_write(&file, WriteOpts::default()).unwrap();
    w.write_all(&data).unwrap();
    w.flush().unwrap();
    drop(w);
    let mut got = Vec::new();
    fs.open_read(&file).unwrap().read_to_end(&mut got).unwrap();
    assert_eq!(got, data);
    assert_eq!(fs.stat(&file).unwrap().size, Some(300_000));
}

#[test]
fn errors_are_typed() {
    let (dir, server, fs) = setup();
    assert!(matches!(
        fs.read_dir(&uri(&fs, "/missing")),
        Err(FmError::NotFound(_))
    ));
    std::fs::write(dir.path().join("f"), b"x").unwrap();
    server.faults.deny_open.store(true, Ordering::Relaxed);
    assert!(matches!(
        fs.open_read(&uri(&fs, "/f")),
        Err(FmError::PermissionDenied(_))
    ));
    server.faults.deny_open.store(false, Ordering::Relaxed);
    // The session stays usable after a refused call.
    assert!(fs.open_read(&uri(&fs, "/f")).is_ok());
    // A uri for another server is refused, not misrouted.
    let other = VfsUri::parse("sftp://bob@elsewhere/x").unwrap();
    assert!(fs.stat(&other).is_err());
}

#[test]
fn hostile_listing_is_filtered_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "pw").unwrap();
    let make = |cap: usize| {
        connect(&params(&server, "pw"), Arc::new(TrustAll)).map(|f| f.with_max_entries(cap))
    };
    server.faults.hostile_entries.store(50, Ordering::Relaxed);

    let fs = make(1000).unwrap();
    let list = fs.read_dir(&uri(&fs, "/")).unwrap();
    assert!(list.iter().all(|e| !e.name.contains('/')));
    assert_eq!(
        list.len(),
        51,
        "50 bogus + the u64::MAX entry; bad name dropped"
    );
    let huge = list.iter().find(|e| e.name == "huge").unwrap();
    assert_eq!(huge.size, Some(u64::MAX));

    let tight = make(10).unwrap();
    let err = tight.read_dir(&uri(&tight, "/")).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}

#[test]
fn space_is_none_without_the_extension() {
    let (_dir, _server, fs) = setup();
    assert_eq!(fs.space(&uri(&fs, "/")).unwrap(), None);
}

#[test]
fn key_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "pw").unwrap();
    let keyfile = dir.path().join("id");
    let pem = client_key()
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap();
    std::fs::write(&keyfile, pem.as_bytes()).unwrap();
    let mut p = params(&server, "");
    p.auth = AuthMethod::Key {
        path: keyfile,
        passphrase: None,
    };
    let fs = connect(&p, Arc::new(TrustAll)).unwrap();
    assert!(fs.read_dir(&uri(&fs, "/")).is_ok());
}

#[test]
fn host_key_verifier_sees_the_fingerprint_and_can_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "pw").unwrap();
    struct Spy(Mutex<Option<HostKeyInfo>>);
    impl HostKeyVerifier for Spy {
        fn verify(&self, info: &HostKeyInfo) -> HostKeyVerdict {
            *self.0.lock().unwrap() = Some(info.clone());
            HostKeyVerdict::Reject("host key was not accepted".into())
        }
    }
    let spy = Arc::new(Spy(Mutex::new(None)));
    let err = connect(&params(&server, "pw"), spy.clone()).unwrap_err();
    assert!(
        err.to_string().contains("host key was not accepted"),
        "{err}"
    );
    let seen = spy.0.lock().unwrap().clone().unwrap();
    assert_eq!(
        seen.fingerprint,
        server
            .host_public
            .fingerprint(russh::keys::HashAlg::Sha256)
            .to_string()
    );
    assert!(seen.fingerprint.starts_with("SHA256:"));
    assert!(seen.openssh.starts_with("ssh-ed25519 "));
}

#[test]
fn failed_connects_never_leak_the_secret() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "right").unwrap();
    let canary = "hunter2-canary";
    let mut errors = Vec::new();

    // Wrong password.
    errors.push(connect(&params(&server, canary), Arc::new(TrustAll)).unwrap_err());
    // Refused host key.
    struct No;
    impl HostKeyVerifier for No {
        fn verify(&self, _: &HostKeyInfo) -> HostKeyVerdict {
            HostKeyVerdict::Reject("nope".into())
        }
    }
    errors.push(connect(&params(&server, canary), Arc::new(No)).unwrap_err());
    // Nothing listening.
    let mut dead = params(&server, canary);
    dead.port = 1;
    errors.push(connect(&dead, Arc::new(TrustAll)).unwrap_err());
    // Invalid params, and an unusable key file with the canary as passphrase.
    let mut bad = params(&server, canary);
    bad.host = "bad host".into();
    errors.push(connect(&bad, Arc::new(TrustAll)).unwrap_err());
    let mut key = params(&server, canary);
    key.auth = AuthMethod::Key {
        path: dir.path().join("missing"),
        passphrase: Some(SecretString::from(canary.to_string())),
    };
    errors.push(connect(&key, Arc::new(TrustAll)).unwrap_err());

    for e in &errors {
        let shown = format!("{e} | {e:?}");
        assert!(!shown.contains(canary), "leaked: {shown}");
    }
    assert!(matches!(errors[0], FmError::PermissionDenied(_)));
    let dbg = format!("{:?}", params(&server, canary));
    assert!(!dbg.contains(canary), "ConnectParams Debug leaked: {dbg}");
}

#[test]
fn dropped_connection_is_a_transport_error() {
    let (_dir, server, fs) = setup();
    assert!(fs.is_alive());
    server.faults.kill_connections();
    let err = (0..50)
        .find_map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(100));
            fs.read_dir(&uri(&fs, "/")).err()
        })
        .expect("listing should fail once the link is gone");
    assert!(is_transport_error(&err), "{err:?}");
    assert!(!fs.is_alive());
}

#[test]
fn calling_from_inside_a_runtime_is_an_error_not_a_deadlock() {
    let (_dir, _server, fs) = setup();
    let u = uri(&fs, "/");
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let err = rt.block_on(async { fs.read_dir(&u) }).unwrap_err();
    assert!(err.to_string().contains("async runtime"), "{err}");
}

#[test]
fn a_slow_server_delays_only_the_calling_thread() {
    let (dir, server, fs) = setup();
    std::fs::write(dir.path().join("f"), b"abc").unwrap();
    server.faults.read_delay_ms.store(300, Ordering::Relaxed);
    let started = std::time::Instant::now();
    let mut got = String::new();
    fs.open_read(&uri(&fs, "/f"))
        .unwrap()
        .read_to_string(&mut got)
        .unwrap();
    assert_eq!(got, "abc");
    assert!(started.elapsed() >= std::time::Duration::from_millis(300));
}

#[test]
fn ppk_keys_get_an_actionable_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = TestServer::start(dir.path(), "alice", "pw").unwrap();
    let mut p = params(&server, "");
    p.auth = AuthMethod::Key {
        path: dir.path().join("id.PPK"),
        passphrase: None,
    };
    let err = connect(&p, Arc::new(TrustAll)).unwrap_err();
    assert!(err.to_string().contains("Export OpenSSH key"), "{err}");
}

#[test]
fn set_permissions_changes_the_remote_mode() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, _server, fs) = setup();
    std::fs::write(dir.path().join("p.sh"), b"x").unwrap();
    fs.set_permissions(&uri(&fs, "/p.sh"), 0o750).unwrap();
    let mode = std::fs::metadata(dir.path().join("p.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o750);
    assert_eq!(
        fs.stat(&uri(&fs, "/p.sh")).unwrap().permissions,
        Some(0o750)
    );
}
