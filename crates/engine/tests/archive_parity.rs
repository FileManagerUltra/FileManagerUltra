//! add-archive-vfs 4.7: archives behave the same on local, SFTP and SMB locations
//! (in-memory backends standing in for them), nested ones included; a compressed tar
//! reports progress while it is opened and stops when the pane leaves it.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Read as _, Write as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::{DirState, OpState, Session};
use fm_testkit::{FaultFs, MemFs};
use fm_vfs::{Scheme, VfsUri};

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    for (name, data) in entries {
        if let Some(dir) = name.strip_suffix('/') {
            w.add_directory(dir, o).expect("dir");
        } else {
            w.start_file(*name, o).expect("f");
            w.write_all(data).expect("w");
        }
    }
    w.finish().expect("finish").into_inner()
}

fn poll_until(
    s: &mut Session,
    what: &str,
    limit: Duration,
    mut done: impl FnMut(&mut Session) -> bool,
) {
    let deadline = Instant::now() + limit;
    loop {
        s.poll();
        if done(s) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn location(s: &Session) -> VfsUri {
    s.layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone()
}

fn names(s: &Session, dir: &VfsUri) -> Vec<String> {
    let mut v: Vec<String> = s
        .dirs
        .entries(dir)
        .expect("listed")
        .iter()
        .map(|e| e.name.clone())
        .collect();
    v.sort();
    v
}

/// Enter the archive file `file` and wait for its listing.
fn enter(s: &mut Session, file: &VfsUri) -> VfsUri {
    s.dispatch("archive.enter", &json!({ "uri": file.to_string() }))
        .unwrap_or_else(|e| panic!("enter {file}: {e}"));
    let root = location(s);
    assert_eq!(root.scheme(), Scheme::Archive);
    poll_until(s, "archive listing", Duration::from_secs(20), |s| {
        s.dirs.state(&root).is_ready()
    });
    root
}

fn read(s: &Session, uri: &VfsUri) -> Vec<u8> {
    let mut out = Vec::new();
    s.backends
        .require(uri)
        .expect("backend")
        .open_read(uri)
        .expect("open")
        .read_to_end(&mut out)
        .expect("read");
    out
}

/// A folder on each kind of backend: local, SFTP and SMB, each an in-memory filesystem.
fn backends() -> Vec<(&'static str, MemFs, VfsUri)> {
    let local = if cfg!(windows) { "C:\\data" } else { "/data" };
    vec![
        ("local", MemFs::new(), VfsUri::parse(local).expect("uri")),
        (
            "sftp",
            MemFs::new().with_scheme(Scheme::Sftp),
            VfsUri::parse("sftp://bob@host/srv/data").expect("uri"),
        ),
        (
            "smb",
            MemFs::new().with_scheme(Scheme::Smb),
            VfsUri::parse("smb://nas/share/data").expect("uri"),
        ),
    ]
}

#[test]
fn archives_browse_nest_and_save_the_same_on_local_sftp_and_smb() {
    let inner = zip_with(&[("deep/", b""), ("deep/note.txt", b"nested")]);
    let outer = zip_with(&[
        ("docs/", b""),
        ("docs/readme", b"hi"),
        ("inner.zip", &inner),
    ]);
    for (kind, mem, folder) in backends() {
        let mut s = Session::new().expect("session");
        mem.mkdirs(&folder).expect("folder");
        let file = folder.join("site.zip").expect("file");
        mem.put_file(&file, &outer).expect("zip");
        if folder.scheme() == Scheme::File {
            s.backends.register(Arc::new(mem.clone()));
        } else {
            s.backends
                .register_at(folder.authority(), Arc::new(mem.clone()));
        }

        // Browse and read a member.
        let root = enter(&mut s, &file);
        assert_eq!(names(&s, &root), vec!["docs", "inner.zip"], "{kind}");
        assert_eq!(
            read(
                &s,
                &root
                    .join("docs")
                    .and_then(|d| d.join("readme"))
                    .expect("uri")
            ),
            b"hi",
            "{kind}"
        );

        // An archive inside it.
        let nested = enter(&mut s, &root.join("inner.zip").expect("uri"));
        assert_eq!(fm_engine::archive_cmds::nesting(&nested), 2, "{kind}");
        assert_eq!(names(&s, &nested), vec!["deep"], "{kind}");
        assert_eq!(
            read(
                &s,
                &nested
                    .join("deep")
                    .and_then(|d| d.join("note.txt"))
                    .expect("uri")
            ),
            b"nested",
            "{kind}"
        );

        // A staged change saved back to the archive file on that backend.
        s.dispatch("nav.open", &json!(root.to_string()))
            .expect("back");
        let vfs = s.backends.require(&root).expect("backend");
        vfs.create_dir(&root.join("new").expect("uri"))
            .expect("mkdir");
        s.dispatch("archive.save", &json!({ "uri": root.to_string() }))
            .unwrap_or_else(|e| panic!("{kind} save: {e}"));
        poll_until(&mut s, "save", Duration::from_secs(20), |s| {
            s.fileops.jobs().iter().all(|j| !j.state.is_running())
        });
        for job in s.fileops.jobs() {
            assert_eq!(job.state, OpState::Finished, "{kind}: {:?}", job.summary);
        }
        let saved = mem.get_file(&file).expect("saved archive");
        let z = zip::ZipArchive::new(Cursor::new(saved)).expect("still a zip");
        let mut entries: Vec<String> = z
            .file_names()
            .map(|n| n.expect("name").into_owned())
            .collect();
        entries.sort_unstable();
        assert_eq!(
            entries,
            vec!["docs/", "docs/readme", "inner.zip", "new/"],
            "{kind}"
        );
    }
}

/// A `.tar.gz` of `files` members with incompressible contents, so reading it takes a
/// measurable number of reads.
fn slow_tar_gz(files: usize) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());
    let mut seed = 0x2545_f491_u32;
    for i in 0..files {
        let data: Vec<u8> = (0..2048)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed.to_le_bytes()[0]
            })
            .collect();
        let mut h = tar::Header::new_gnu();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, format!("files/f{i:05}.bin"), &data[..])
            .expect("append");
    }
    let tar = b.into_inner().expect("tar");
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).expect("gz");
    gz.finish().expect("gz")
}

#[test]
fn a_compressed_tar_shows_progress_and_stops_when_the_pane_leaves_it() {
    let folder = VfsUri::parse(if cfg!(windows) { "C:\\data" } else { "/data" }).expect("uri");
    let mem = MemFs::new();
    mem.mkdirs(&folder).expect("folder");
    let file = folder.join("dump.tar.gz").expect("file");
    mem.put_file(&file, &slow_tar_gz(1500)).expect("tar.gz");
    // Small, slow reads: the whole archive takes seconds to read.
    let slow = FaultFs::new(mem);
    {
        let mut faults = slow.faults();
        faults.read_chunk = Some(4096);
        faults.read_hook = Some(Arc::new(|| std::thread::sleep(Duration::from_millis(3))));
    }
    let mut s = Session::new().expect("session");
    s.backends.register(Arc::new(slow.clone()));
    s.dispatch("nav.open", &json!(folder.to_string()))
        .expect("folder");

    s.dispatch("archive.enter", &json!({ "uri": file.to_string() }))
        .expect("enter");
    let root = location(&s);
    poll_until(&mut s, "progress", Duration::from_secs(10), |s| {
        fm_engine::archive_fill::reading(s, &root).is_some_and(|r| r.percent.is_some_and(|p| p > 0))
    });
    let reading = fm_engine::archive_fill::reading(&s, &root).expect("reading");
    assert!(
        reading.percent.is_some_and(|p| p < 100),
        "{:?}",
        reading.percent
    );
    assert_eq!(reading.name, "dump.tar.gz");

    // Leaving the archive stops reading it, well before the whole file was read.
    let left = Instant::now();
    s.dispatch("nav.open", &json!(folder.to_string()))
        .expect("leave");
    poll_until(&mut s, "the open to stop", Duration::from_secs(5), |s| {
        matches!(s.dirs.state(&root), DirState::Failed { .. })
    });
    assert!(left.elapsed() < Duration::from_secs(3), "stopped promptly");
    let mount = s.archives.for_uri(&root).expect("mount").clone();
    assert!(mount.opening_for().is_none(), "no longer opening");
    assert!(mount.opened().is_none(), "not opened");

    // Entering again reads it from the start, this time to the end.
    slow.faults().read_hook = None;
    s.dispatch("archive.enter", &json!({ "uri": file.to_string() }))
        .expect("enter again");
    poll_until(&mut s, "the full listing", Duration::from_secs(20), |s| {
        s.dirs.state(&root).is_ready()
    });
    let files = root.join("files").expect("uri");
    s.ensure_read(files.clone()).expect("read files");
    poll_until(&mut s, "files", Duration::from_secs(20), |s| {
        s.dirs.state(&files).is_ready()
    });
    assert_eq!(s.dirs.entries(&files).expect("entries").len(), 1500);
}

#[test]
fn an_archive_opened_only_for_the_tree_is_not_stopped() {
    let folder = VfsUri::parse(if cfg!(windows) { "C:\\data" } else { "/data" }).expect("uri");
    let mem = MemFs::new();
    mem.mkdirs(&folder).expect("folder");
    let file = folder.join("dump.tar.gz").expect("file");
    mem.put_file(&file, &slow_tar_gz(300)).expect("tar.gz");
    let slow = FaultFs::new(mem);
    {
        let mut faults = slow.faults();
        faults.read_chunk = Some(4096);
        faults.read_hook = Some(Arc::new(|| std::thread::sleep(Duration::from_millis(2))));
    }
    let mut s = Session::new().expect("session");
    s.backends.register(Arc::new(slow));
    s.dispatch("nav.open", &json!(folder.to_string()))
        .expect("folder");
    // Expanded in the tree (all archives as folders), the pane staying in the folder.
    let root = fm_engine::archive_cmds::tree_root(&file).expect("tree root");
    let pane = s.layout.focused();
    if let Some(state) = s.layout.state_mut(pane) {
        state.tree.expand(root.clone());
    }
    s.ensure_read(root.clone()).expect("tree read");
    poll_until(&mut s, "the tree's listing", Duration::from_secs(30), |s| {
        !matches!(
            s.dirs.state(&root),
            DirState::Loading { .. } | DirState::Absent
        )
    });
    assert!(s.dirs.state(&root).is_ready(), "{:?}", s.dirs.state(&root));
    assert_eq!(location(&s), folder, "the pane never entered it");
}

/// Double-clicking an archive opens it in the pane it was clicked in, also in an SFTP or
/// Network pane: no second pane appears (the plan and dispatch the app uses).
#[test]
fn double_clicking_an_archive_in_a_remote_pane_opens_it_there_only() {
    use fm_engine::open_cmds::{OpenPlan, plan};
    let outer = zip_with(&[("docs/", b""), ("docs/readme", b"hi")]);
    for (kind, mem, folder) in backends() {
        let mut s = Session::new().expect("session");
        mem.mkdirs(&folder).expect("folder");
        let file = folder.join("site.zip").expect("file");
        mem.put_file(&file, &outer).expect("zip");
        if folder.scheme() == Scheme::File {
            s.backends.register(Arc::new(mem));
        } else {
            s.backends.register_at(folder.authority(), Arc::new(mem));
        }
        let pane = s.layout.focused();
        {
            let st = s.layout.state_mut(pane).expect("pane");
            st.kind = match folder.scheme() {
                Scheme::Sftp => fm_layout::PaneKind::Sftp,
                Scheme::Smb => fm_layout::PaneKind::Network,
                _ => fm_layout::PaneKind::Files,
            };
            st.navigate(folder.clone());
        }
        s.ensure_read(folder.clone()).expect("read");
        poll_until(&mut s, "listing", Duration::from_secs(10), |s| {
            s.dirs.state(&folder).is_ready()
        });
        let before = s.layout.panes().len();
        let planned = plan(&mut s, &file);
        match planned {
            OpenPlan::Launch => s
                .dispatch("file.open", &json!({ "uri": file.to_string() }))
                .expect("open"),
            OpenPlan::Navigate => s
                .dispatch("nav.open", &json!(file.to_string()))
                .expect("nav"),
            other => panic!("{kind}: unexpected plan {other:?}"),
        }
        for _ in 0..50 {
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(s.layout.panes().len(), before, "{kind}: no extra pane");
        assert_eq!(s.layout.focused(), pane, "{kind}: same pane focused");
        assert_eq!(location(&s).scheme(), Scheme::Archive, "{kind}: entered");
    }
}
