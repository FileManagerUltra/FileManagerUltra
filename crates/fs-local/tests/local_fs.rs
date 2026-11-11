//! Tempdir round-trip suite for the local backend (capability `local-backend`).

use std::fs;
use std::io::{Read, Write};

use fm_fs_local::LocalFs;
#[cfg(unix)]
use fm_vfs::EntryKind;
use fm_vfs::{FmError, Vfs, VfsUri, WriteOpts};

#[allow(clippy::expect_used)] // test helpers
fn uri(path: &std::path::Path) -> VfsUri {
    VfsUri::parse(&format!("file://{}", path.display())).expect("tempdir uri parses")
}

#[allow(clippy::expect_used)] // test helpers
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn full_trait_walk_over_a_nested_tree() {
    let tmp = tempdir();
    let root = tmp.path();
    fs::create_dir(root.join("docs")).expect("mkdir docs");
    fs::create_dir(root.join("docs/2026")).expect("mkdir nested");
    fs::write(root.join("docs/readme.txt"), b"hello").expect("write");
    fs::write(root.join("docs/2026/log.bin"), vec![0u8; 1024]).expect("write");
    let fs_ = LocalFs::new();

    let root_uri = uri(root);
    let mut entries = fs_.read_dir(&root_uri).expect("read root");
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["docs"]);
    assert!(entries[0].is_dir());

    let docs_uri = root_uri.join("docs").expect("join");
    let mut docs = fs_.read_dir(&docs_uri).expect("read docs");
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(docs.len(), 2);
    assert!(docs[0].is_dir()); // 2026 first (dirs_first is the engine's job; here it's raw)
    assert!(docs[1].is_file());

    // stat: file with size and mtime
    let file_uri = docs_uri.join("readme.txt").expect("join");
    let st = fs_.stat(&file_uri).expect("stat file");
    assert!(st.is_file());
    assert_eq!(st.size, Some(5));
    assert!(st.modified.is_some());
    assert!(st.permissions.is_some() || cfg!(windows));

    // open_read
    let mut r = fs_.open_read(&file_uri).expect("open read");
    let mut buf = String::new();
    r.read_to_string(&mut buf).expect("read");
    assert_eq!(buf, "hello");

    // open_write append + truncate
    let mut w = fs_
        .open_write(
            &file_uri,
            WriteOpts {
                create: false,
                truncate: false,
                append: true,
            },
        )
        .expect("append");
    w.write_all(b" world").expect("append write");
    drop(w);
    let mut r2 = fs_.open_read(&file_uri).expect("reopen");
    let mut buf2 = String::new();
    r2.read_to_string(&mut buf2).expect("read");
    assert_eq!(buf2, "hello world");

    let mut w2 = fs_
        .open_write(&file_uri, WriteOpts::default())
        .expect("trunc");
    w2.write_all(b"x").expect("write");
    w2.flush().expect("flush");
    drop(w2);
    assert_eq!(fs_.stat(&file_uri).expect("stat").size, Some(1));

    // open_write create
    let new_uri = docs_uri.join("created.txt").expect("join");
    let mut w3 = fs_
        .open_write(
            &new_uri,
            WriteOpts {
                create: true,
                truncate: true,
                append: false,
            },
        )
        .expect("create");
    w3.write_all(b"new").expect("write");
    drop(w3);
    assert!(fs_.stat(&new_uri).expect("stat new").is_file());

    // create_dir
    let fresh = docs_uri.join("fresh").expect("join");
    fs_.create_dir(&fresh).expect("mkdir");
    assert!(fs_.stat(&fresh).expect("stat fresh").is_dir());

    // rename file
    let renamed = docs_uri.join("renamed.txt").expect("join");
    fs_.rename(&file_uri, &renamed).expect("rename");
    assert!(fs_.stat(&file_uri).is_err());
    assert!(fs_.stat(&renamed).expect("stat renamed").is_file());

    // remove file, then the now-empty dir
    fs_.remove(&renamed).expect("remove file");
    fs_.remove(&fresh).expect("remove empty dir");

    // space (best-effort: Some on Linux, None tolerated elsewhere)
    let sp = fs_.space(&root_uri).expect("space must not error");
    if let Some(info) = sp {
        assert!(info.total >= info.available);
    }

    // same_device within the tempdir is true on unix
    #[cfg(unix)]
    assert!(fs_.same_device(&root_uri, &docs_uri));
}

#[test]
fn typed_errors_for_the_common_mistakes() {
    let tmp = tempdir();
    let root = tmp.path();
    let fs_ = LocalFs::new();
    let root_uri = uri(root);

    // missing path
    let missing = root_uri.join("nope").expect("join");
    assert!(matches!(fs_.stat(&missing), Err(FmError::NotFound(_))));
    assert!(matches!(fs_.read_dir(&missing), Err(FmError::NotFound(_))));

    // file where a directory is expected
    let file = root_uri.join("f.txt").expect("join");
    fs_.open_write(&file, WriteOpts::default())
        .expect("create file");
    assert!(matches!(
        fs_.read_dir(&file),
        Err(FmError::NotADirectory(_))
    ));

    // non-empty directory removal
    let dir = root_uri.join("d").expect("join");
    fs_.create_dir(&dir).expect("mkdir");
    let inner = dir.join("inner.txt").expect("join");
    fs_.open_write(&inner, WriteOpts::default())
        .expect("create inner");
    assert!(matches!(fs_.remove(&dir), Err(FmError::NotEmpty(_))));
    fs_.remove(&inner).expect("remove inner");
    fs_.remove(&dir).expect("remove now-empty dir");

    // rename dir onto an existing *non-empty* dir → typed error (ENOTEMPTY)
    let a = root_uri.join("a").expect("join");
    let b = root_uri.join("b").expect("join");
    fs::create_dir(root.join("a")).expect("mkdir a");
    fs::write(root.join("a/x"), b"x").expect("write x");
    fs::create_dir(root.join("b")).expect("mkdir b");
    fs::write(root.join("b/y"), b"y").expect("write y");
    assert!(fs_.rename(&a, &b).is_err());

    // vanished between read_dir and stat
    let ghost = root_uri.join("ghost.txt").expect("join");
    fs::write(root.join("ghost.txt"), b"g").expect("write");
    fs_.read_dir(&root_uri).expect("read");
    fs::remove_file(root.join("ghost.txt")).expect("unlink");
    assert!(matches!(fs_.stat(&ghost), Err(FmError::NotFound(_))));
}

#[cfg(unix)]
#[test]
fn permission_denied_is_typed_when_not_root() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempdir();
    let root = tmp.path();
    let locked = root.join("locked");
    fs::create_dir(&locked).expect("mkdir");
    fs::write(locked.join("f"), b"x").expect("write");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("chmod");
    let fs_ = LocalFs::new();
    let err = fs_.read_dir(&uri(&locked));
    // Root bypasses mode bits; the assertion only holds for a real user.
    if err.is_ok() {
        let _ = fs::set_permissions(&locked, fs::Permissions::from_mode(0o755));
        eprintln!("skipping: running with privileges that ignore mode bits");
        return;
    }
    assert!(matches!(err, Err(FmError::PermissionDenied(_))), "{err:?}");
    let _ = fs::set_permissions(&locked, fs::Permissions::from_mode(0o755));
}

#[cfg(unix)]
#[test]
fn special_files_report_other_and_are_never_regular() {
    // /dev/null is a char device on every unix.
    let dev_null = VfsUri::parse("file:///dev/null").expect("static uri");
    let fs_ = LocalFs::new();
    let st = fs_.stat(&dev_null).expect("stat /dev/null");
    assert_eq!(st.kind, EntryKind::Other);
    assert!(!st.is_file());
    assert!(!st.is_dir());
}

#[test]
fn over_the_entry_cap_fails_actionably() {
    let tmp = tempdir();
    let root = tmp.path();
    for i in 0..4 {
        fs::write(root.join(format!("f{i}.txt")), b"x").expect("write");
    }
    let fs_ = LocalFs::with_max_entries(3);
    let err = fs_.read_dir(&uri(root)).expect_err("must cap");
    let msg = err.to_string();
    assert!(msg.contains("too large"), "{msg}");
    assert!(msg.contains('3'), "{msg}");
    // under the cap it succeeds
    let ok = LocalFs::with_max_entries(4).read_dir(&uri(root));
    assert!(ok.is_ok());
}

#[cfg(unix)]
#[test]
fn non_utf8_names_render_lossily_without_panic() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let tmp = tempdir();
    let root = tmp.path();
    let weird = OsStr::from_bytes(b"bad\xff.txt");
    fs::write(root.join(weird), b"x").expect("write weird");
    let fs_ = LocalFs::new();
    let entries = fs_.read_dir(&uri(root)).expect("read");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].name.contains("bad"), "{}", entries[0].name);
    assert!(!entries[0].name.contains('\u{fffd}') || entries[0].name.contains('.'));
}

#[test]
fn open_write_refuses_to_create_when_asked_not_to() {
    let tmp = tempdir();
    let root = tmp.path();
    let fs_ = LocalFs::new();
    let missing = uri(root).join("nope.bin").expect("join");
    let err = fs_.open_write(
        &missing,
        WriteOpts {
            create: false,
            truncate: true,
            append: false,
        },
    );
    assert!(err.is_err());
}

#[test]
fn read_write_roundtrip_through_the_trait_objects() {
    // Guards the Box<dyn Read + Send>/Write plumbing end to end.
    let tmp = tempdir();
    let root = tmp.path();
    let fs_ = LocalFs::new();
    let u = uri(root).join("blob.bin").expect("join");
    let mut w = fs_.open_write(&u, WriteOpts::default()).expect("create");
    w.write_all(b"0123456789").expect("write");
    w.flush().expect("flush");
    drop(w);
    let mut r = fs_.open_read(&u).expect("open");
    let mut all = Vec::new();
    r.read_to_end(&mut all).expect("read all");
    assert_eq!(all, b"0123456789");
}

#[cfg(unix)]
#[test]
#[allow(clippy::expect_used)]
fn create_symlink_stores_the_target_verbatim_and_never_follows_it() {
    let tmp = tempdir();
    let root = tmp.path();
    fs::write(root.join("real.txt"), b"data").expect("write");
    let fs_ = LocalFs::new();

    let link = uri(&root.join("link"));
    fs_.create_symlink("real.txt", &link).expect("create");
    fs_.create_symlink("/does/not/exist", &uri(&root.join("dangling")))
        .expect("dangling");
    assert_eq!(
        fs::read_link(root.join("link")).expect("read_link"),
        std::path::Path::new("real.txt")
    );
    assert!(matches!(
        fs_.stat(&link).expect("stat").kind,
        EntryKind::Symlink { ref target } if target == "real.txt"
    ));
    assert!(matches!(
        fs_.create_symlink("x", &link),
        Err(FmError::AlreadyExists(_))
    ));
    fs_.remove(&link).expect("remove link");
    assert!(
        root.join("real.txt").exists(),
        "removing a link keeps its target"
    );
}

#[test]
#[allow(clippy::expect_used)]
fn links_to_directories_are_flagged_openable() {
    let tmp = tempdir();
    let root = tmp.path();
    fs::create_dir(root.join("realdir")).expect("mkdir");
    fs::write(root.join("file.txt"), b"x").expect("write");
    let fs_ = LocalFs::new();
    // Skip silently where the OS refuses links (Windows without the privilege).
    if fs_
        .create_symlink("realdir", &uri(&root.join("dlink")))
        .is_err()
        || fs_
            .create_symlink("file.txt", &uri(&root.join("flink")))
            .is_err()
        || fs_
            .create_symlink("nowhere", &uri(&root.join("broken")))
            .is_err()
    {
        return;
    }
    let by_name = |n: &str| fs_.stat(&uri(&root.join(n))).expect("stat");
    assert!(by_name("dlink").link_to_dir && by_name("dlink").is_dir_like());
    assert!(!by_name("flink").link_to_dir);
    assert!(!by_name("broken").link_to_dir);
    assert!(!by_name("realdir").link_to_dir && by_name("realdir").is_dir_like());
}

#[cfg(unix)]
#[test]
#[allow(clippy::expect_used)]
fn set_permissions_changes_the_mode_but_refuses_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let file = dir.path().join("run.sh");
    fs::write(&file, b"#!/bin/sh").expect("write");
    let vfs = LocalFs::new();
    vfs.set_permissions(&uri(&file), 0o100_755).expect("chmod");
    let mode = fs::metadata(&file).expect("meta").permissions().mode();
    assert_eq!(mode & 0o7777, 0o755, "file-type bits are masked off");
    assert_eq!(
        vfs.stat(&uri(&file)).expect("stat").permissions,
        Some(0o755)
    );
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&file, &link).expect("symlink");
    assert!(matches!(
        vfs.set_permissions(&uri(&link), 0o700),
        Err(FmError::Unsupported(_))
    ));
    assert!(matches!(
        vfs.set_permissions(&uri(&dir.path().join("gone")), 0o700),
        Err(FmError::NotFound(_))
    ));
}
