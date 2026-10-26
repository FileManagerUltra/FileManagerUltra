#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests against a throwaway Samba container (`dperson/samba`). Every test
//! skips (passes) when Docker or the image is unavailable.

use std::io::{Read, Write};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use fm_fs_smb::{ConnectParams, SecretString, SmbFs, connect, is_transport_error};
use fm_vfs::{EntryKind, FmError, Vfs, VfsUri, WriteOpts};

const USER: &str = "alice";
const PASS: &str = "s3cret";
const SHARE: &str = "share";
/// Tests that use the container; the last one to finish removes it.
const USERS: usize = 5;

static FINISHED: AtomicUsize = AtomicUsize::new(0);

struct Samba {
    name: String,
    port: u16,
}

fn docker(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn start() -> Option<Samba> {
    docker(&["image", "inspect", "dperson/samba"])?;
    let name = format!("fm-smb-test-{}", std::process::id());
    docker(&[
        "run",
        "-d",
        "--name",
        &name,
        "-p",
        "127.0.0.1::445",
        "dperson/samba",
        "-u",
        &format!("{USER};{PASS}"),
        "-s",
        &format!("{SHARE};/share;yes;no;no;{USER};{USER};{USER}"),
        "-s",
        &format!("other;/other;yes;no;no;{USER};{USER};{USER}"),
        "-p",
    ])?;
    let mapped = docker(&["port", &name, "445/tcp"])?;
    let port: u16 = mapped.lines().next()?.rsplit(':').next()?.parse().ok()?;
    let samba = Samba { name, port };
    for _ in 0..40 {
        if connect(&params(&samba, PASS)).is_ok() {
            return Some(samba);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

fn server() -> Option<&'static Samba> {
    static S: OnceLock<Option<Samba>> = OnceLock::new();
    S.get_or_init(start).as_ref()
}

/// Held by each container-using test; the last drop removes the container.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        if FINISHED.fetch_add(1, Ordering::SeqCst) + 1 == USERS
            && let Some(s) = server()
        {
            let _ = docker(&["rm", "-f", &s.name]);
        }
    }
}

fn params(s: &Samba, pass: &str) -> ConnectParams {
    ConnectParams {
        host: "127.0.0.1".into(),
        port: Some(s.port),
        share: SHARE.into(),
        user: USER.into(),
        domain: None,
        password: SecretString::from(pass.to_string()),
        timeout: Duration::from_secs(10),
        require_encryption: false,
        anonymous_uri: false,
    }
}

fn uri(fs: &SmbFs, path: &str) -> VfsUri {
    VfsUri::parse(&format!("smb://{}/{SHARE}{path}", fs.authority())).unwrap()
}

macro_rules! server_or_skip {
    () => {{
        let _guard = Guard;
        let Some(s) = server() else {
            eprintln!("skipped: no docker / dperson/samba");
            return;
        };
        (_guard, s)
    }};
}

#[test]
fn everything_roundtrips() {
    let (_g, s) = server_or_skip!();
    let fs = connect(&params(s, PASS)).unwrap();
    let dir = uri(&fs, "/rt");
    fs.create_dir(&dir).unwrap();
    assert!(matches!(
        fs.create_dir(&dir),
        Err(FmError::AlreadyExists(_))
    ));

    // 3 MiB + 7 bytes: several chunks, an odd tail.
    let data: Vec<u8> = (0..3 * 1024 * 1024 + 7).map(|i| (i % 251) as u8).collect();
    let big = uri(&fs, "/rt/big.bin");
    let mut w = fs.open_write(&big, WriteOpts::default()).unwrap();
    w.write_all(&data).unwrap();
    w.flush().unwrap();
    drop(w);

    let e = fs.stat(&big).unwrap();
    assert_eq!(e.size, Some(data.len() as u64));
    assert!(matches!(e.kind, EntryKind::File));
    assert!(e.modified.is_some());

    let mut back = Vec::new();
    fs.open_read(&big).unwrap().read_to_end(&mut back).unwrap();
    assert_eq!(back, data);

    let mut tail = Vec::new();
    fs.open_read_at(&big, 3 * 1024 * 1024)
        .unwrap()
        .read_to_end(&mut tail)
        .unwrap();
    assert_eq!(tail, data[3 * 1024 * 1024..]);

    let mut app = fs
        .open_write(
            &big,
            WriteOpts {
                create: false,
                truncate: false,
                append: true,
            },
        )
        .unwrap();
    app.write_all(b"END").unwrap();
    drop(app);
    assert_eq!(fs.stat(&big).unwrap().size, Some(data.len() as u64 + 3));

    let listing = fs.read_dir(&dir).unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "big.bin");
    let top = fs.read_dir(&uri(&fs, "")).unwrap();
    assert!(
        top.iter()
            .any(|e| e.name == "rt" && e.kind == EntryKind::Dir)
    );

    assert!(matches!(fs.remove(&dir), Err(FmError::NotEmpty(_))));

    let moved = uri(&fs, "/rt/moved.bin");
    fs.rename(&big, &moved).unwrap();
    assert!(matches!(fs.stat(&big), Err(FmError::NotFound(_))));
    let other = uri(&fs, "/rt/other.bin");
    drop(fs.open_write(&other, WriteOpts::default()).unwrap());
    assert!(matches!(
        fs.rename(&other, &moved),
        Err(FmError::AlreadyExists(_))
    ));

    let space = fs.space(&dir).unwrap().unwrap();
    assert!(space.total > 0 && space.available <= space.total);

    fs.remove(&moved).unwrap();
    fs.remove(&other).unwrap();
    fs.remove(&dir).unwrap();
    assert!(matches!(fs.remove(&dir), Err(FmError::NotFound(_))));
    assert!(fs.is_alive());
}

#[test]
fn hostile_and_foreign_uris_are_rejected() {
    let (_g, s) = server_or_skip!();
    let fs = connect(&params(s, PASS)).unwrap();
    let other_share = VfsUri::parse(&format!("smb://{}/other/x", fs.authority())).unwrap();
    assert!(fs.stat(&other_share).is_err());
    let other_host = VfsUri::parse(&format!("smb://bob@elsewhere/{SHARE}/x")).unwrap();
    assert!(fs.read_dir(&other_host).is_err());
    assert!(fs.remove(&uri(&fs, "")).is_err());
    assert!(fs.rename(&uri(&fs, ""), &uri(&fs, "/x")).is_err());
    assert!(fs.open_read(&uri(&fs, "/does/not/exist")).is_err());
}

#[test]
fn listing_is_capped() {
    let (_g, s) = server_or_skip!();
    let fs = connect(&params(s, PASS)).unwrap().with_max_entries(2);
    let dir = uri(&fs, "/cap");
    fs.create_dir(&dir).unwrap();
    let names = ["a", "b", "c", "d"];
    for n in names {
        drop(
            fs.open_write(&uri(&fs, &format!("/cap/{n}")), WriteOpts::default())
                .unwrap(),
        );
    }
    let err = fs.read_dir(&dir).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    for n in names {
        fs.remove(&uri(&fs, &format!("/cap/{n}"))).unwrap();
    }
    fs.remove(&dir).unwrap();
}

#[test]
fn bad_credentials_fail_without_leaking_the_password() {
    let (_g, s) = server_or_skip!();
    let canary = "hunter2-canary";
    let mut errors = vec![connect(&params(s, canary)).unwrap_err()];
    let mut bad_share = params(s, PASS);
    bad_share.share = "nope".into();
    errors.push(connect(&bad_share).unwrap_err());
    let mut bad_host = params(s, canary);
    bad_host.host = "bad host".into();
    errors.push(connect(&bad_host).unwrap_err());
    let mut dead = params(s, canary);
    dead.port = Some(1);
    dead.timeout = Duration::from_secs(2);
    errors.push(connect(&dead).unwrap_err());
    for e in &errors {
        let text = format!("{e} {e:?}");
        assert!(!text.contains(canary), "leaked: {text}");
    }
    assert!(
        errors[0]
            .to_string()
            .to_lowercase()
            .contains("authentication")
    );
    assert!(is_transport_error(&errors[3]) || matches!(errors[3], FmError::Backend { .. }));
}

#[test]
fn debug_of_params_hides_the_password() {
    let p = ConnectParams {
        host: "h".into(),
        port: None,
        share: "s".into(),
        user: "u".into(),
        domain: None,
        password: SecretString::from("hunter2-canary".to_string()),
        timeout: Duration::from_secs(1),
        require_encryption: false,
        anonymous_uri: false,
    };
    assert!(!format!("{p:?}").contains("hunter2-canary"));
}

#[test]
fn other_shares_connect_on_demand() {
    let (_g, s) = server_or_skip!();
    let fs = connect(&params(s, PASS)).unwrap();
    let root = VfsUri::parse(&format!("smb://{}/", fs.authority())).unwrap();
    let names = |fs: &SmbFs| -> Vec<String> {
        fs.read_dir(&root)
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect()
    };
    // The root lists what the server offers, not just what is connected.
    assert_eq!(names(&fs), ["other", SHARE]);
    assert_eq!(fs.stat(&root).unwrap().kind, EntryKind::Dir);
    assert!(fs.space(&root).unwrap().is_none());
    assert!(fs.create_dir(&root).is_err());

    let other = VfsUri::parse(&format!("smb://{}/other/f.txt", fs.authority())).unwrap();
    drop(fs.open_write(&other, WriteOpts::default()).unwrap());
    assert_eq!(names(&fs), ["other", SHARE]);
    assert!(!fs.same_device(&other, &uri(&fs, "/x")));
    assert!(fs.same_device(&other, &other));
    assert!(matches!(
        fs.rename(&other, &uri(&fs, "/f.txt")),
        Err(FmError::Unsupported(_))
    ));
    fs.remove(&other).unwrap();

    let missing = VfsUri::parse(&format!("smb://{}/nosuchshare/x", fs.authority())).unwrap();
    assert!(fs.read_dir(&missing).is_err());
    assert_eq!(names(&fs), ["other", SHARE]);

    // Signing in without a share starts at the server root and lists the same shares.
    let mut sign_in = params(s, PASS);
    sign_in.share = String::new();
    let bare = connect(&sign_in).unwrap();
    assert!(bare.shares().is_empty());
    assert_eq!(names(&bare), ["other", SHARE]);
    assert_eq!(bare.start_uri().unwrap(), root);
}
