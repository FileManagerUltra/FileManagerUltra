//! Extracting hostile archives (add-archive-vfs 6.2, 6.3): nothing is written outside the
//! destination, special entries are left out and listed, setuid bits never survive, a
//! member larger than its header says stops, and a bomb-like ratio asks first.
#![allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{Cursor, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::{OpState, Session};
use fm_vfs::VfsUri;

fn uri(p: &Path) -> String {
    VfsUri::parse(p.to_str().expect("utf8"))
        .expect("uri")
        .to_string()
}

fn poll_until(s: &mut Session, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        s.poll();
        if done(s) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_idle(s: &mut Session) {
    poll_until(s, "jobs to finish", |s| {
        s.fileops.jobs().iter().all(|j| !j.state.is_running())
    });
}

/// Every path under `root`, relative, sorted (links are not followed).
fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).expect("ls").flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .expect("rel")
                .to_string_lossy()
                .replace('\\', "/");
            let meta = std::fs::symlink_metadata(&path).expect("meta");
            out.push(rel);
            if meta.is_dir() {
                walk(base, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// A tar entry with a raw (unvalidated) name, as a hostile tool would write it.
fn raw_entry(
    b: &mut tar::Builder<Vec<u8>>,
    name: &str,
    kind: tar::EntryType,
    mode: u32,
    data: &[u8],
    link: Option<&str>,
) {
    let mut h = tar::Header::new_gnu();
    let gnu = h.as_gnu_mut().expect("gnu");
    gnu.name[..name.len()].copy_from_slice(name.as_bytes());
    if let Some(l) = link {
        gnu.linkname[..l.len()].copy_from_slice(l.as_bytes());
    }
    h.set_entry_type(kind);
    h.set_mode(mode);
    h.set_size(data.len() as u64);
    h.set_mtime(1_700_000_000);
    h.set_cksum();
    b.append(&h, data).expect("append");
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    outside: PathBuf,
    work: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("tmp");
    let root = dir.path().to_path_buf();
    let outside = root.join("outside");
    let work = root.join("a/b/work");
    std::fs::create_dir_all(&outside).expect("outside");
    std::fs::create_dir_all(&work).expect("work");
    std::fs::write(outside.join("sentinel"), b"keep").expect("sentinel");
    Fixture {
        _dir: dir,
        root,
        outside,
        work,
    }
}

fn hostile_tar(outside: &Path) -> Vec<u8> {
    use tar::EntryType as T;
    let mut b = tar::Builder::new(Vec::new());
    raw_entry(&mut b, "docs/a.txt", T::Regular, 0o644, b"alpha", None);
    raw_entry(&mut b, "../../.bashrc", T::Regular, 0o644, b"evil", None);
    raw_entry(
        &mut b,
        "../../../outside/planted",
        T::Regular,
        0o644,
        b"x",
        None,
    );
    raw_entry(&mut b, "/abs/passwd", T::Regular, 0o644, b"root", None);
    raw_entry(&mut b, "C:\\win\\x.txt", T::Regular, 0o644, b"win", None);
    raw_entry(&mut b, "dev/sda", T::Block, 0o660, b"", None);
    raw_entry(&mut b, "fifo", T::Fifo, 0o644, b"", None);
    raw_entry(&mut b, "hard-out", T::Link, 0o644, b"", Some("/etc/passwd"));
    raw_entry(&mut b, "hard-in", T::Link, 0o644, b"", Some("docs/a.txt"));
    raw_entry(&mut b, "suid.sh", T::Regular, 0o6755, b"#!/bin/sh\n", None);
    let target = outside.to_str().expect("utf8");
    raw_entry(&mut b, "escape", T::Symlink, 0o777, b"", Some(target));
    raw_entry(
        &mut b,
        "escape/pwned.txt",
        T::Regular,
        0o644,
        b"pwned",
        None,
    );
    raw_entry(&mut b, "rel", T::Symlink, 0o777, b"", Some("../../outside"));
    b.into_inner().expect("tar")
}

#[test]
fn a_hostile_tar_extracts_only_inside_its_folder() {
    let f = fixture();
    std::fs::write(f.work.join("hostile.tar"), hostile_tar(&f.outside)).expect("tar");
    let before_outside = tree(&f.outside);
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.extract",
        &json!({"uri": uri(&f.work.join("hostile.tar"))}),
    )
    .expect("extract");
    wait_idle(&mut s);
    let job = s.fileops.jobs().last().expect("job");
    assert_eq!(job.state, OpState::Finished, "{:?}", job.summary);
    let out = f.work.join("hostile");
    let got = tree(&out);
    let mut want = vec![
        ".bashrc",
        "abs",
        "abs/passwd",
        "dev",
        "docs",
        "docs/a.txt",
        "escape",
        "escape/pwned.txt",
        "hard-in",
        "outside",
        "outside/planted",
        "suid.sh",
        "win",
        "win/x.txt",
    ];
    // Windows creates the link only with Developer Mode or as administrator; otherwise
    // it is reported as failed with a message saying so (checked below).
    let rel_made = std::fs::symlink_metadata(out.join("rel")).is_ok();
    if cfg!(unix) || rel_made {
        want.push("rel");
    }
    want.sort_unstable();
    assert_eq!(got, want, "everything lands inside the new folder");
    // Nothing was written next to, above or outside the destination.
    assert_eq!(tree(&f.outside), before_outside, "outside untouched");
    assert_eq!(
        tree(&f.work),
        vec!["hostile.tar".to_string()]
            .into_iter()
            .chain(want.iter().map(|w| format!("hostile/{w}")))
            .chain(std::iter::once("hostile".to_string()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    for dir in [f.root.join("a"), f.root.join("a/b")] {
        assert!(!dir.join(".bashrc").exists(), "{}", dir.display());
    }
    assert_eq!(std::fs::read(out.join(".bashrc")).expect("bashrc"), b"evil");
    assert_eq!(std::fs::read(out.join("hard-in")).expect("hard"), b"alpha");
    // The link named like a folder lost to the folder: nothing went through it.
    assert!(
        std::fs::symlink_metadata(out.join("escape"))
            .expect("escape")
            .is_dir()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(out.join("suid.sh"))
            .expect("suid")
            .permissions()
            .mode();
        assert_eq!(mode & 0o6000, 0, "setuid/setgid cleared: {mode:o}");
        let rel = std::fs::read_link(out.join("rel")).expect("link stays a link");
        assert_eq!(rel, Path::new("../../outside"));
    }
    let summary = job.summary.as_ref().expect("summary");
    let left: Vec<&str> = summary
        .left_out
        .iter()
        // Display paths use `\` on Windows.
        .map(|(item, _)| item.rsplit(['/', '\\']).next().unwrap_or(item))
        .collect();
    for name in ["sda", "fifo", "hard-out"] {
        assert!(left.contains(&name), "{name} listed as left out: {left:?}");
    }
    assert!(summary.skipped >= 3, "{summary:?}");
    // Only the link Windows refused may fail, and only with the actionable message.
    let failed: Vec<_> = summary
        .failed
        .iter()
        .filter(|(item, why)| {
            let rel_refused = !rel_made
                && cfg!(windows)
                && item.ends_with("/rel")
                && why.contains("Developer Mode");
            !rel_refused
        })
        .collect();
    assert!(failed.is_empty(), "{:?}", summary.failed);
}

/// A deflated zip whose central directory and local header claim `declared` bytes for
/// the member `name`.
fn zip_with_declared(name: &str, data: &[u8], declared: u32) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    w.start_file("ok.txt", opts).expect("start");
    w.write_all(b"fine").expect("w");
    w.start_file(name, opts).expect("start");
    w.write_all(data).expect("w");
    let mut bytes = w.finish().expect("finish").into_inner();
    let patch =
        |bytes: &mut Vec<u8>, sig: [u8; 4], size_at: usize, name_len_at: usize, fixed: usize| {
            let mut i = 0;
            while i + fixed <= bytes.len() {
                if bytes[i..i + 4] == sig {
                    let n = u16::from_le_bytes([bytes[i + name_len_at], bytes[i + name_len_at + 1]])
                        as usize;
                    if bytes.get(i + fixed..i + fixed + n) == Some(name.as_bytes()) {
                        bytes[i + size_at..i + size_at + 4]
                            .copy_from_slice(&declared.to_le_bytes());
                    }
                }
                i += 1;
            }
        };
    // Central directory header: uncompressed size at 24, name length at 28, name at 46.
    patch(&mut bytes, [0x50, 0x4b, 0x01, 0x02], 24, 28, 46);
    // Local file header: uncompressed size at 22, name length at 26, name at 30.
    patch(&mut bytes, [0x50, 0x4b, 0x03, 0x04], 22, 26, 30);
    bytes
}

#[test]
fn a_member_larger_than_its_header_says_stops_without_a_partial_file() {
    let f = fixture();
    let zeros = vec![0u8; 300_000];
    std::fs::write(
        f.work.join("liar.zip"),
        zip_with_declared("big.bin", &zeros, 1000),
    )
    .expect("zip");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.extract",
        &json!({"uri": uri(&f.work.join("liar.zip"))}),
    )
    .expect("extract");
    wait_idle(&mut s);
    let job = s.fileops.jobs().last().expect("job");
    let summary = job.summary.as_ref().expect("summary");
    assert_eq!(summary.failed.len(), 1, "{summary:?}");
    // Either the zip crate's own check or ours (`Declared`) stops it.
    assert!(
        summary.failed[0].1.contains("larger than its"),
        "{:?}",
        summary.failed
    );
    let out = f.work.join("liar");
    assert_eq!(tree(&out), vec!["ok.txt"], "no partial or temp file");
}

#[test]
fn a_bomb_like_ratio_asks_first_and_no_leaves_nothing() {
    let f = fixture();
    // ~1 KB on disk that claims 3.75 GiB: far above 1 GiB at more than 1000:1.
    let bomb = zip_with_declared("zeros.bin", &[0u8; 4096], 0xF000_0000);
    std::fs::write(f.work.join("bomb.zip"), &bomb).expect("zip");
    let mut s = Session::new().expect("session");
    assert!(!s.command_enabled("job.answer"));
    assert!(
        s.dispatch("job.answer", &json!({"job": 1, "go": true}))
            .is_err(),
        "no question to answer"
    );
    s.dispatch(
        "archive.extract",
        &json!({"uri": uri(&f.work.join("bomb.zip"))}),
    )
    .expect("extract");
    poll_until(&mut s, "the bomb question", |s| {
        s.fileops.pending_question().is_some()
    });
    let job = s.fileops.pending_question().expect("question");
    let id = job.id;
    let text = job.question.clone().expect("text");
    assert!(text.contains("decompression bomb"), "{text}");
    assert!(text.contains("bomb.zip") || text.contains("bomb"), "{text}");
    assert!(s.command_enabled("job.answer"));
    for bad in [
        json!({"job": id}),
        json!({"job": id, "go": "yes"}),
        json!({"go": true}),
    ] {
        assert!(s.dispatch("job.answer", &bad).is_err(), "{bad}");
    }
    s.dispatch("job.answer", &json!({"job": id, "go": false}))
        .expect("answer");
    wait_idle(&mut s);
    let job = s.fileops.get(id).expect("job");
    assert_eq!(job.state, OpState::Cancelled);
    assert!(job.question.is_none());
    assert_eq!(
        tree(&f.work),
        vec!["bomb.zip"],
        "declining leaves no folder and no file"
    );

    // Answering yes goes on: the folder is made and the job ends on its own.
    s.dispatch(
        "archive.extract",
        &json!({"uri": uri(&f.work.join("bomb.zip"))}),
    )
    .expect("extract again");
    poll_until(&mut s, "the bomb question again", |s| {
        s.fileops.pending_question().is_some()
    });
    let id = s.fileops.pending_question().expect("question").id;
    s.dispatch("job.answer", &json!({"job": id, "go": true}))
        .expect("answer yes");
    wait_idle(&mut s);
    assert!(f.work.join("bomb").is_dir());
    assert!(f.work.join("bomb/ok.txt").is_file());
}

fn aes_zip(password: &str) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .with_aes_encryption(zip::AesMode::Aes256, password);
    w.start_file("docs/a.txt", opts).expect("start");
    w.write_all(b"alpha").expect("w");
    w.finish().expect("finish").into_inner()
}

fn last_state(s: &Session) -> OpState {
    s.fileops.jobs().last().expect("job").state.clone()
}

#[test]
fn extracting_an_encrypted_zip_asks_then_runs_again_with_the_password() {
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    let mut s = Session::new().expect("session");
    assert!(!s.command_enabled("archive.password.cancel"));
    s.dispatch("archive.extract", &json!({"uri": uri(&zip)}))
        .expect("extract");
    wait_idle(&mut s);
    assert!(matches!(last_state(&s), OpState::Failed(m) if m.contains("needs a password")));
    assert_eq!(
        tree(&f.work),
        vec!["secret.zip"],
        "nothing written before asking"
    );
    let ask = s
        .archives
        .password_ask
        .clone()
        .expect("asks for the password");
    assert!(!ask.wrong);
    assert!(s.command_enabled("archive.password.cancel"));

    // A wrong password: the job runs again, stops again, and the dialog says so.
    s.dispatch(
        "archive.password",
        &json!({"uri": ask.uri.to_string(), "password": "nope"}),
    )
    .expect("wrong password");
    wait_idle(&mut s);
    assert_eq!(s.fileops.jobs().len(), 2, "the job ran again");
    let ask = s.archives.password_ask.clone().expect("asks again");
    assert!(ask.wrong, "says the password was wrong");
    assert_eq!(tree(&f.work), vec!["secret.zip"]);

    s.dispatch(
        "archive.password",
        &json!({"uri": ask.uri.to_string(), "password": "right"}),
    )
    .expect("right password");
    wait_idle(&mut s);
    assert_eq!(last_state(&s), OpState::Finished);
    assert!(s.archives.password_ask.is_none());
    assert_eq!(
        std::fs::read(f.work.join("secret/docs/a.txt")).expect("extracted"),
        b"alpha"
    );
}

#[test]
fn three_wrong_passwords_stop_asking_until_forgotten() {
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    let mut s = Session::new().expect("session");
    s.dispatch("archive.extract", &json!({"uri": uri(&zip)}))
        .expect("extract");
    wait_idle(&mut s);
    let root = s.archives.password_ask.clone().expect("ask").uri;
    for (i, pw) in ["one", "two", "three"].iter().enumerate() {
        s.dispatch(
            "archive.password",
            &json!({"uri": root.to_string(), "password": pw}),
        )
        .expect("password");
        wait_idle(&mut s);
        let asked = s.archives.password_ask.is_some();
        assert_eq!(asked, i < 2, "after wrong password {}", i + 1);
    }
    assert!(
        matches!(last_state(&s), OpState::Failed(m) if m.contains("Forget Password")),
        "{:?}",
        last_state(&s)
    );
    let refused = s.dispatch(
        "archive.password",
        &json!({"uri": root.to_string(), "password": "right"}),
    );
    assert!(refused.is_err(), "no fourth try without forgetting");
    s.dispatch("archive.password.forget", &json!({"uri": root.to_string()}))
        .expect("forget");
    s.dispatch("archive.extract", &json!({"uri": uri(&zip)}))
        .expect("extract again");
    wait_idle(&mut s);
    assert!(
        s.archives.password_ask.is_some(),
        "asks again after forgetting"
    );
    s.dispatch(
        "archive.password",
        &json!({"uri": root.to_string(), "password": "right"}),
    )
    .expect("right");
    wait_idle(&mut s);
    assert_eq!(last_state(&s), OpState::Finished);
}

#[test]
fn cancelling_the_password_dialog_drops_the_retry() {
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    let mut s = Session::new().expect("session");
    assert!(s.dispatch("archive.password.cancel", &json!(null)).is_err());
    s.dispatch("archive.extract", &json!({"uri": uri(&zip)}))
        .expect("extract");
    wait_idle(&mut s);
    assert!(s.archives.password_ask.is_some());
    assert!(
        s.dispatch("archive.password.cancel", &json!({"x": 1}))
            .is_err()
    );
    s.dispatch("archive.password.cancel", &json!(null))
        .expect("cancel");
    assert!(s.archives.password_ask.is_none());
    assert_eq!(s.fileops.jobs().len(), 1, "nothing ran again");
}

#[test]
fn encrypted_entries_are_known_for_the_lock_badge() {
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    let mut s = Session::new().expect("session");
    s.dispatch("archive.enter", &json!({"uri": uri(&zip)}))
        .expect("enter");
    let root = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    let docs = root.join("docs").expect("docs");
    poll_until(&mut s, "the archive to open", |s| {
        s.archives.opened_fs(&root).is_some()
    });
    assert!(s.archives.is_encrypted(&docs.join("a.txt").expect("a")));
    assert!(!s.archives.is_encrypted(&docs), "folders carry no lock");
}

#[test]
fn a_preview_inside_an_encrypted_zip_shows_after_the_password() {
    use fm_engine::preview_cmds::Body;
    use fm_layout::PaneKind;
    use fm_preview::Preview;
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    let mut s = Session::new().expect("session");
    s.dispatch("archive.enter", &json!({"uri": uri(&zip)}))
        .expect("enter");
    let root = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    let file = root
        .join("docs")
        .and_then(|d| d.join("a.txt"))
        .expect("file");
    let docs = root.join("docs").expect("docs");
    s.dispatch("nav.open", &json!(docs.to_string()))
        .expect("into docs");
    poll_until(&mut s, "the docs listing", |s| {
        s.dirs.state(&docs).is_ready()
    });
    s.dispatch("preview.open", &json!({"uri": file.to_string()}))
        .expect("preview");
    let pane = s
        .layout
        .panes()
        .into_iter()
        .find(|id| s.pane_kind(*id) == Some(PaneKind::Preview))
        .expect("preview pane");
    let body_text = |s: &Session| match s.preview_slot(pane).map(|slot| &slot.body) {
        Some(Body::Static(p)) => match &**p {
            Preview::Error(e) => Some(format!("error: {e}")),
            Preview::Text(_) => Some("text".to_string()),
            other => Some(format!("{other:?}").chars().take(40).collect()),
        },
        _ => None,
    };
    poll_until(&mut s, "the password error", |s| {
        body_text(s).is_some_and(|t| t.starts_with("error:"))
    });
    let shown = body_text(&s).expect("body");
    assert!(
        fm_engine::archive_cmds::is_password_message(shown.trim_start_matches("error: ")),
        "{shown}"
    );
    s.dispatch(
        "archive.password",
        &json!({"uri": file.to_string(), "password": "right"}),
    )
    .expect("password");
    poll_until(&mut s, "the text after the password", |s| {
        body_text(s).as_deref() == Some("text")
    });
    assert!(s.dirs.state(&docs).is_ready(), "the listing came back too");
}

#[test]
fn any_part_of_a_split_archive_opens_and_extracts_the_whole_set() {
    let f = fixture();
    let mut noise = Vec::with_capacity(300_000);
    let mut x = 0x9e37_79b9_u32;
    for _ in 0..300_000 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        noise.push(x as u8);
    }
    std::fs::write(f.work.join("big.bin"), &noise).expect("big");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.compress",
        &json!({"sources": [uri(&f.work.join("big.bin"))], "target": uri(&f.work.join("backup.7z")), "format": "7z", "split": 100_000}),
    )
    .expect("split");
    wait_idle(&mut s);
    std::fs::remove_file(f.work.join("big.bin")).expect("rm");
    assert!(f.work.join("backup.7z.003").is_file());

    let root_of = |s: &mut Session, part: &str| {
        s.dispatch("archive.enter", &json!({"uri": uri(&f.work.join(part))}))
            .expect("enter");
        s.layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone()
    };
    let from_third = root_of(&mut s, "backup.7z.003");
    poll_until(&mut s, "the set's listing", |s| {
        s.dirs.state(&from_third).is_ready()
    });
    let from_first = root_of(&mut s, "backup.7z.001");
    assert_eq!(from_third, from_first, "every part opens the same mount");
    let shown = fm_engine::item_cmds::display_path(&from_third);
    assert!(shown.ends_with("backup.7z.001"), "{shown}");

    s.dispatch(
        "archive.extract",
        &json!({"uri": uri(&f.work.join("backup.7z.002"))}),
    )
    .expect("extract");
    wait_idle(&mut s);
    assert_eq!(last_state(&s), OpState::Finished);
    assert_eq!(
        std::fs::read(f.work.join("backup/big.bin")).expect("extracted"),
        noise
    );

    // Without part 2 the set does not open, and says which part is missing.
    std::fs::rename(f.work.join("backup.7z.002"), f.root.join("elsewhere")).expect("mv");
    s.dispatch(
        "archive.unmount",
        &json!({"uri": uri(&f.work.join("backup.7z.001"))}),
    )
    .expect("close");
    let root = root_of(&mut s, "backup.7z.001");
    poll_until(&mut s, "the failed listing", |s| {
        matches!(s.dirs.state(&root), fm_engine::DirState::Failed { .. })
    });
    let fm_engine::DirState::Failed { message, .. } = s.dirs.state(&root) else {
        panic!("the listing did not fail")
    };
    assert!(
        message.contains("Part 2 of backup.7z is missing (backup.7z.002)"),
        "{message}"
    );
}

#[test]
fn adding_to_an_encrypted_zip_asks_for_the_password_then_saves_encrypted() {
    let f = fixture();
    let zip = f.work.join("secret.zip");
    std::fs::write(&zip, aes_zip("right")).expect("zip");
    std::fs::write(f.work.join("new.txt"), b"fresh").expect("new");
    let before = std::fs::read(&zip).expect("bytes");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": [uri(&f.work.join("new.txt"))]}),
    )
    .expect("add");
    wait_idle(&mut s);
    assert!(matches!(last_state(&s), OpState::Failed(m) if m.contains("needs a password")));
    assert_eq!(std::fs::read(&zip).expect("bytes"), before, "unchanged");
    let ask = s.archives.password_ask.clone().expect("asks");
    s.dispatch(
        "archive.password",
        &json!({"uri": ask.uri.to_string(), "password": "right"}),
    )
    .expect("password");
    wait_idle(&mut s);
    assert_eq!(last_state(&s), OpState::Finished);
    let file = std::fs::File::open(&zip).expect("open");
    let mut z = zip::ZipArchive::new(file).expect("zip");
    assert!(
        z.by_name("new.txt").is_err(),
        "an encrypted member needs the password"
    );
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut z.by_name_decrypt("new.txt", b"right").expect("decrypt"),
        &mut text,
    )
    .expect("read");
    assert_eq!(text, "fresh");
}
