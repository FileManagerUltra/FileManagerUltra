//! Staged editing inside archives (add-archive-vfs 5.1–5.3): the normal file commands
//! change only the overlay; the archive file is untouched until Save.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::{OpState, Session};
use fm_vfs::VfsUri;

fn zip_bytes() -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default();
    w.add_directory("docs/", o).expect("dir");
    w.start_file("docs/old.txt", o).expect("f");
    w.write_all(b"old").expect("w");
    w.start_file("readme", o).expect("f");
    w.write_all(b"hi").expect("w");
    w.finish().expect("finish").into_inner()
}

fn uri(p: &Path) -> String {
    VfsUri::parse(p.to_str().expect("utf8"))
        .expect("uri")
        .to_string()
}

fn poll_until(s: &mut Session, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        s.poll();
        if done(s) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_jobs(s: &mut Session) {
    poll_until(s, "jobs", |s| {
        s.fileops.jobs().iter().all(|j| !j.state.is_running())
    });
    for j in s.fileops.jobs() {
        assert_eq!(j.state, OpState::Finished, "{}: {:?}", j.label, j.summary);
    }
}

fn names(s: &Session, dir: &VfsUri) -> Vec<String> {
    let vfs = s.backends.require(dir).expect("backend");
    let mut v: Vec<String> = vfs
        .read_dir(dir)
        .expect("list")
        .into_iter()
        .map(|e| e.name)
        .collect();
    v.sort();
    v
}

#[test]
fn file_commands_inside_a_zip_are_staged_and_the_file_is_untouched() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    std::fs::create_dir_all(p.join("src/pics")).expect("src");
    std::fs::write(p.join("src/report.pdf"), b"pdf").expect("pdf");
    std::fs::write(p.join("src/pics/a.png"), b"png").expect("png");
    let before = std::fs::metadata(&zip).expect("meta");

    let mut s = Session::new().expect("session");
    s.dispatch("archive.enter", &json!({"uri": uri(&zip)}))
        .expect("enter");
    let root = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    poll_until(&mut s, "listing", |s| s.dirs.state(&root).is_ready());
    let docs = root.join("docs").expect("docs");

    // Copy a file and a folder in, as one job.
    s.dispatch(
        "file.copy",
        &json!({"sources": [uri(&p.join("src/report.pdf")), uri(&p.join("src/pics"))], "dest": docs.to_string()}),
    )
    .expect("copy");
    wait_jobs(&mut s);
    assert_eq!(names(&s, &docs), vec!["old.txt", "pics", "report.pdf"]);
    let vfs = s.backends.require(&docs).expect("backend");
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut vfs
            .open_read(
                &docs
                    .join("pics")
                    .and_then(|d| d.join("a.png"))
                    .expect("png"),
            )
            .expect("open"),
        &mut text,
    )
    .expect("read");
    assert_eq!(text, "png");

    // New folder, rename, move within the archive, delete (staged, no trash).
    s.dispatch(
        "file.mkdir",
        &json!({"parent": root.to_string(), "name": "new"}),
    )
    .expect("mkdir");
    wait_jobs(&mut s);
    s.dispatch(
        "file.rename",
        &json!({"uri": root.join("readme").expect("r").to_string(), "name": "README.md"}),
    )
    .expect("rename");
    wait_jobs(&mut s);
    s.dispatch(
        "file.move",
        &json!({"sources": [docs.join("old.txt").expect("o").to_string()], "dest": root.join("new").expect("n").to_string()}),
    )
    .expect("move");
    wait_jobs(&mut s);
    s.dispatch(
        "file.delete",
        &json!({"sources": [docs.join("report.pdf").expect("r").to_string()]}),
    )
    .expect("delete is staged");
    wait_jobs(&mut s);
    assert_eq!(names(&s, &root), vec!["README.md", "docs", "new"]);
    assert_eq!(names(&s, &docs), vec!["pics"]);
    assert_eq!(names(&s, &root.join("new").expect("n")), vec!["old.txt"]);

    let after = std::fs::metadata(&zip).expect("meta");
    assert_eq!(
        (after.len(), after.modified().ok()),
        (before.len(), before.modified().ok()),
        "the archive file is not written before Save"
    );
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(mount.is_dirty());
    let changes = mount.opened().expect("open").changes();
    assert!(changes.len() >= 4, "{changes:?}");

    // Closing would lose the changes: refused until saved or discarded.
    assert!(
        s.dispatch("archive.unmount", &json!({"uri": root.to_string()}))
            .is_err()
    );
    assert!(
        s.dispatch("archive.password.forget", &json!({"uri": root.to_string()}))
            .is_err()
    );
}

/// Write `bytes` as `name.001`, `name.002`… of `size` bytes each (the last may be shorter).
fn write_parts(dir: &Path, name: &str, bytes: &[u8], size: usize) -> Vec<String> {
    bytes
        .chunks(size)
        .enumerate()
        .map(|(i, chunk)| {
            let part = format!("{name}.{:03}", i + 1);
            std::fs::write(dir.join(&part), chunk).expect("part");
            part
        })
        .collect()
}

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in entries {
        w.start_file(*name, o).expect("f");
        w.write_all(data).expect("w");
    }
    w.finish().expect("finish").into_inner()
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn a_split_zip_is_saved_back_as_parts_of_the_original_size() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("a.txt", &[7u8; 300]), ("b.txt", &[9u8; 300])]);
    let size = old.len() / 3 + 1;
    let parts = write_parts(p, "set.zip", &old, size);
    assert_eq!(parts.len(), 3);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let vfs = s.backends.require(&root).expect("backend");
    let mut w = vfs
        .open_write(
            &root.join("big.bin").expect("b"),
            fm_vfs::WriteOpts::default(),
        )
        .expect("write");
    // Stored zip entries keep their size, so the set grows by about this much.
    let big: Vec<u8> = (0..2000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    w.write_all(&big).expect("w");
    w.flush().expect("flush");
    drop(w);
    vfs.remove(&root.join("a.txt").expect("a")).expect("rm");
    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("save");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );

    let files = files_in(p);
    assert!(files.len() > 3, "the set grew: {files:?}");
    assert!(
        files.iter().all(|f| f.starts_with("set.zip.0")),
        "{files:?}"
    );
    let mut joined = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let bytes = std::fs::read(p.join(f)).expect("part");
        if i + 1 < files.len() {
            assert_eq!(bytes.len(), size, "{f}");
        } else {
            assert!(!bytes.is_empty() && bytes.len() <= size, "{f}");
        }
        joined.extend(bytes);
    }
    let full = p.join("joined.zip");
    std::fs::write(&full, joined).expect("joined");
    assert_eq!(zip_names(&full), vec!["b.txt", "big.bin"]);
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(!mount.is_dirty());
    // The pane reads the new set and a second save sees nothing staged.
    poll_until(&mut s, "reread", |s| s.dirs.state(&root).is_ready());
    assert_eq!(names(&s, &root), vec!["b.txt", "big.bin"]);
}

#[test]
fn a_split_set_changed_on_disk_is_not_overwritten() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("a.txt", &[7u8; 300])]);
    write_parts(p, "set.zip", &old, old.len() / 2 + 1);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");
    // Another program changes only the last part.
    let last = p.join("set.zip.002");
    let mut bytes = std::fs::read(&last).expect("read");
    bytes.push(0);
    std::fs::write(&last, &bytes).expect("rewrite");
    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("submitted");
    let states = wait_done(&mut s);
    assert!(
        matches!(states.last(), Some(OpState::Failed(m)) if m.contains("changed on disk")),
        "{states:?}"
    );
    assert_eq!(std::fs::read(&last).expect("read"), bytes, "untouched");
    assert_eq!(files_in(p), vec!["set.zip.001", "set.zip.002"]);
    assert!(s.archives.for_uri(&root).expect("mount").is_dirty());
}

#[test]
fn a_split_copy_keeps_the_part_size() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("a.txt", &[7u8; 300])]);
    let size = old.len() / 2 + 1;
    write_parts(p, "set.zip", &old, size);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");
    let out = p.join("out");
    std::fs::create_dir(&out).expect("out");
    s.dispatch(
        "archive.save_as",
        &json!({"uri": root.to_string(), "dest": uri(&out)}),
    )
    .expect("save as");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    let copy = files_in(&out);
    assert!(
        copy.len() >= 2 && copy.iter().all(|f| f.starts_with("set.zip.0")),
        "numbered parts named by the base: {copy:?}"
    );
    let mut joined = Vec::new();
    for (i, f) in copy.iter().enumerate() {
        let bytes = std::fs::read(out.join(f)).expect("part");
        if i + 1 < copy.len() {
            assert_eq!(bytes.len(), size, "{f} has the original part size");
        } else {
            assert!(!bytes.is_empty() && bytes.len() <= size, "{f}");
        }
        joined.extend(bytes);
    }
    let full = p.join("joined.zip");
    std::fs::write(&full, joined).expect("joined");
    assert_eq!(zip_names(&full), vec!["a.txt", "mine/"]);
    std::fs::remove_file(&full).expect("rm");
    assert_eq!(
        files_in(p),
        vec!["out", "set.zip.001", "set.zip.002"],
        "set untouched"
    );
}

#[test]
fn backup_parts_of_an_interrupted_save_make_the_set_read_only_until_resolved() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("old.txt", &[1u8; 300])]);
    let new = zip_with(&[("new.txt", &[2u8; 300])]);
    let size = old.len() / 2 + 1;
    for (i, chunk) in old.chunks(size).enumerate() {
        std::fs::write(p.join(format!(".set.zip.{:03}.fmbak", i + 1)), chunk).expect("bak");
    }
    write_parts(p, "set.zip", &new, size);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let vfs = s.backends.require(&root).expect("backend");
    assert_eq!(
        names(&s, &root),
        vec!["new.txt"],
        "the parts in place are listed"
    );
    assert!(
        vfs.create_dir(&root.join("x").expect("x")).is_err(),
        "read-only"
    );
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(
        s.dispatch("archive.backup.restore", &json!({"uri": "/tmp"}))
            .is_err()
    );

    // Restoring puts the old set back and drops the new parts.
    s.dispatch("archive.backup.restore", &json!({"uri": root.to_string()}))
        .expect("restore");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    assert_eq!(files_in(p), vec!["set.zip.001", "set.zip.002"]);
    let mut joined = std::fs::read(p.join("set.zip.001")).expect("1");
    joined.extend(std::fs::read(p.join("set.zip.002")).expect("2"));
    assert_eq!(joined, old);
    poll_until(&mut s, "reread", |s| s.dirs.state(&root).is_ready());
    assert_eq!(names(&s, &root), vec!["old.txt"]);
    assert!(mount.opened().is_some_and(|fs| fs.writable().is_ok()));
}

#[test]
fn an_interrupted_swap_with_a_missing_first_part_is_restored_from_any_part() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("old.txt", &[1u8; 300])]);
    let size = old.len() / 2 + 1;
    for (i, chunk) in old.chunks(size).enumerate() {
        std::fs::write(p.join(format!(".set.zip.{:03}.fmbak", i + 1)), chunk).expect("bak");
    }
    // Only the second new part is in place: the set has a gap and cannot be opened.
    std::fs::write(p.join("set.zip.002"), b"new").expect("new");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.backup.restore",
        &json!({"uri": uri(&p.join("set.zip.002"))}),
    )
    .expect("restore");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    assert_eq!(files_in(p), vec!["set.zip.001", "set.zip.002"]);
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    assert_eq!(names(&s, &root), vec!["old.txt"]);
}

#[test]
fn deleting_backup_parts_keeps_the_new_set() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let new = zip_with(&[("new.txt", &[2u8; 300])]);
    write_parts(p, "set.zip", &new, new.len() / 2 + 1);
    std::fs::write(p.join(".set.zip.001.fmbak"), b"old1").expect("bak");
    std::fs::write(p.join(".set.zip.002.fmbak"), b"old2").expect("bak");
    std::fs::write(p.join(".set.zip.003.fmbak"), b"old3").expect("bak");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.backup.discard",
        &json!({"uri": uri(&p.join("set.zip.001"))}),
    )
    .expect("discard");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    assert_eq!(files_in(p), vec!["set.zip.001", "set.zip.002"]);
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    assert_eq!(names(&s, &root), vec!["new.txt"]);
    let vfs = s.backends.require(&root).expect("backend");
    assert!(
        vfs.create_dir(&root.join("x").expect("x")).is_ok(),
        "writable again"
    );
}

fn open_zip(s: &mut Session, zip: &Path) -> VfsUri {
    s.dispatch("archive.enter", &json!({"uri": uri(zip)}))
        .expect("enter");
    let root = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    poll_until(s, "listing", |s| s.dirs.state(&root).is_ready());
    root
}

fn zip_names(path: &Path) -> Vec<String> {
    let file = std::fs::File::open(path).expect("open");
    let mut z = zip::ZipArchive::new(file).expect("zip");
    let mut v: Vec<String> = (0..z.len())
        .map(|i| z.by_index(i).expect("e").name().expect("name").to_string())
        .collect();
    v.sort();
    v
}

fn wait_done(s: &mut Session) -> Vec<OpState> {
    poll_until(s, "jobs", |s| {
        s.fileops.jobs().iter().all(|j| !j.state.is_running())
    });
    s.fileops.jobs().iter().map(|j| j.state.clone()).collect()
}

#[test]
fn save_writes_the_staged_changes_and_clears_them() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("new").expect("n"))
        .expect("mkdir");
    let mut w = vfs
        .open_write(
            &root
                .join("new")
                .and_then(|d| d.join("report.pdf"))
                .expect("r"),
            fm_vfs::WriteOpts::default(),
        )
        .expect("write");
    w.write_all(b"pdf").expect("w");
    w.flush().expect("flush");
    drop(w);
    vfs.remove(&root.join("readme").expect("r")).expect("rm");
    // A pane inside the new folder stays there after the save.
    s.dispatch("nav.open", &json!(root.join("new").expect("n").to_string()))
        .expect("nav");

    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("save");
    assert!(
        wait_done(&mut s).iter().all(|st| *st == OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    assert_eq!(
        zip_names(&zip),
        vec!["docs/", "docs/old.txt", "new/", "new/report.pdf"]
    );
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(!mount.is_dirty(), "saved: nothing staged any more");
    let here = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    assert_eq!(here, root.join("new").expect("n"));
    poll_until(&mut s, "reread", |s| s.dirs.state(&here).is_ready());
    assert_eq!(names(&s, &here), vec!["report.pdf"]);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".fmtmp") || n.contains(".fmbak"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    // Nothing left to save.
    assert!(
        s.dispatch("archive.save", &json!({"uri": root.to_string()}))
            .is_err()
    );
}

#[test]
fn an_archive_changed_on_disk_is_not_overwritten() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");
    // Another program rewrites the archive.
    let mut other = zip_bytes();
    other.extend_from_slice(b"trailing");
    std::fs::write(&zip, &other).expect("rewrite");
    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("submitted");
    let states = wait_done(&mut s);
    assert!(
        matches!(states.last(), Some(OpState::Failed(m)) if m.contains("changed on disk")),
        "{states:?}"
    );
    assert_eq!(std::fs::read(&zip).expect("read"), other, "untouched");
    assert!(
        s.archives.for_uri(&root).expect("mount").is_dirty(),
        "changes kept"
    );

    // Save as copy still works, into another folder, and clears the staged set.
    let out = dir.path().join("out");
    std::fs::create_dir(&out).expect("out");
    s.dispatch(
        "archive.save_as",
        &json!({"uri": root.to_string(), "dest": uri(&out)}),
    )
    .expect("save as");
    let states = wait_done(&mut s);
    assert_eq!(
        states.last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    assert_eq!(
        zip_names(&out.join("site.zip")),
        vec!["docs/", "docs/old.txt", "mine/", "readme"]
    );
    assert_eq!(
        std::fs::read(&zip).expect("read"),
        other,
        "original untouched"
    );
    assert!(!s.archives.for_uri(&root).expect("mount").is_dirty());
}

#[test]
fn discard_drops_the_changes_and_leaves_staged_folders() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("tmp").expect("t"))
        .expect("mkdir");
    vfs.remove(&root.join("readme").expect("r")).expect("rm");
    s.dispatch("nav.open", &json!(root.join("tmp").expect("t").to_string()))
        .expect("nav");
    assert!(
        s.dispatch("archive.discard", &json!({"uri": root.to_string()}))
            .is_err(),
        "needs confirm"
    );
    s.dispatch("archive.changes", &json!({"uri": root.to_string()}))
        .expect("changes");
    assert_eq!(s.archives.changes_shown.as_ref(), Some(&root));
    s.dispatch(
        "archive.discard",
        &json!({"uri": root.to_string(), "confirm": true}),
    )
    .expect("discard");
    assert!(!s.archives.for_uri(&root).expect("mount").is_dirty());
    assert_eq!(
        s.layout.state(s.layout.focused()).expect("pane").location,
        root,
        "left the staged folder"
    );
    poll_until(&mut s, "reread", |s| s.dirs.state(&root).is_ready());
    assert_eq!(names(&s, &root), vec!["docs", "readme"]);
}

/// Quitting with a changed workspace and an archive with staged changes asks once, listing
/// both; Save saves both and then quits (add-archive-vfs 5.9, one prompt for everything).
#[test]
fn quitting_asks_once_for_workspaces_and_archives_and_saves_both() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    s.dispatch("workspace.new", &json!(null)).expect("new tab");
    s.dispatch("workspace.save", &json!({"name": "Work"}))
        .expect("save ws");
    let root = open_zip(&mut s, &zip);
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    assert!(!s.workspace_unsaved().is_empty(), "the workspace changed");
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");

    s.dispatch("app.quit", &json!(null)).expect("quit");
    let prompt = s.unsaved_prompt.clone().expect("one prompt");
    assert_eq!(prompt.names, vec!["Work".to_string()]);
    assert_eq!(prompt.archives.len(), 1);
    assert_eq!(
        prompt.archives.first().map(|a| a.1.as_str()),
        Some("site.zip")
    );
    assert!(s.running);

    s.dispatch("workspace.prompt", &json!({"choice": "save"}))
        .expect("save all");
    assert!(s.unsaved_prompt.is_none(), "no second prompt");
    poll_until(&mut s, "quit after saving", |s| !s.running);
    assert!(s.workspace_unsaved().is_empty(), "workspace saved");
    assert!(
        zip_names(&zip).contains(&"mine/".to_string()),
        "archive saved"
    );
}

/// Closing one of two panes showing an archive with staged changes does not ask; closing
/// the last one does, and Don't save drops the changes.
#[test]
fn closing_the_last_pane_on_a_dirty_archive_asks() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split again");
    let panes = s.layout.panes();
    // Two panes show the archive, the third goes elsewhere.
    let (a, b, c) = (panes[0], panes[1], panes[2]);
    let elsewhere = uri(dir.path());
    s.layout.focus_pane(c).expect("focus");
    s.dispatch("nav.open", &json!(elsewhere)).expect("away");
    for p in [a, b] {
        s.layout.focus_pane(p).expect("focus");
        s.dispatch("nav.open", &json!(root.to_string()))
            .expect("archive");
    }
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");

    s.dispatch("pane.close", &json!({"pane": a}))
        .expect("close one");
    assert!(s.unsaved_prompt.is_none(), "another pane still shows it");
    assert_eq!(s.layout.panes().len(), 2);
    s.dispatch("pane.close", &json!({"pane": b}))
        .expect("close last");
    let prompt = s.unsaved_prompt.clone().expect("asks");
    assert_eq!(prompt.archives.len(), 1);
    assert!(prompt.busy.is_empty() && prompt.names.is_empty());
    s.dispatch("workspace.prompt", &json!({"choice": "discard"}))
        .expect("don't save");
    assert_eq!(s.layout.panes().len(), 1, "closed");
    assert!(
        !s.archives.for_uri(&root).expect("mount").is_dirty(),
        "changes dropped"
    );
}

/// Set Password is a staged change; Save encrypts every entry and the archive keeps
/// reading without asking (the mount knows the new password).
#[test]
fn a_password_set_on_a_zip_is_applied_on_save() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    for bad in [
        json!({"password": ""}),
        json!({"password": 3}),
        json!({"encrypt_names": "yes", "password": "x"}),
    ] {
        assert!(s.dispatch("archive.password.set", &bad).is_err(), "{bad}");
    }
    s.dispatch(
        "archive.password.set",
        &json!({"uri": root.to_string(), "password": "s3cret"}),
    )
    .expect("stage");
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(mount.is_dirty());
    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("save");
    assert_eq!(
        wait_done(&mut s).last(),
        Some(&OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    let mut z = zip::ZipArchive::new(std::fs::File::open(&zip).expect("open")).expect("zip");
    for i in 0..z.len() {
        let e = z.by_index_raw(i).expect("entry");
        assert!(e.is_dir() || e.encrypted());
    }
    let vfs = s.backends.require(&root).expect("backend");
    let mut text = String::new();
    std::io::Read::read_to_string(
        &mut vfs
            .open_read(&root.join("readme").expect("r"))
            .expect("open"),
        &mut text,
    )
    .expect("read without asking");
    assert_eq!(text, "hi");
    // Removing it again is offered and staged.
    s.dispatch("archive.password.remove", &json!({"uri": root.to_string()}))
        .expect("remove");
    assert!(mount.is_dirty());
}

fn add_sources(p: &Path, n: usize) -> Vec<String> {
    std::fs::create_dir_all(p.join("src/pics")).expect("src");
    std::fs::write(p.join("src/pics/a.png"), b"png").expect("png");
    let mut sources = vec![uri(&p.join("src/pics"))];
    for i in 0..n {
        let f = p.join(format!("src/f{i}.txt"));
        std::fs::write(&f, format!("file {i}")).expect("file");
        sources.push(uri(&f));
    }
    sources
}

#[test]
fn add_stages_a_selection_and_saves_it_in_the_same_job() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let sources = add_sources(p, 7);
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": sources}),
    )
    .expect("add");
    assert_eq!(s.fileops.jobs().len(), 1, "one job, not stage plus save");
    assert!(
        wait_done(&mut s).iter().all(|st| *st == OpState::Finished),
        "{:?}",
        s.fileops.jobs()
    );
    let names = zip_names(&zip);
    assert_eq!(names.len(), 3 + 2 + 7, "{names:?}");
    for n in ["pics/", "pics/a.png", "f0.txt", "f6.txt", "readme"] {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    assert!(
        fm_engine::archive_cmds::dirty_handles(&s).is_empty(),
        "nothing left staged"
    );
}

#[test]
fn add_asks_about_existing_names_once_and_replaces_on_overwrite() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    std::fs::create_dir_all(p.join("src")).expect("src");
    std::fs::write(p.join("src/readme"), b"new readme").expect("readme");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": [uri(&p.join("src/readme"))]}),
    )
    .expect("add");
    poll_until(&mut s, "conflict", |s| {
        s.fileops.pending_conflict().is_some()
    });
    let id = s.fileops.jobs().first().expect("job").id;
    s.dispatch(
        "job.resolve_conflict",
        &json!({"job": id, "choice": "overwrite"}),
    )
    .expect("resolve");
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    let file = std::fs::File::open(&zip).expect("open");
    let mut z = zip::ZipArchive::new(file).expect("zip");
    let mut text = String::new();
    std::io::Read::read_to_string(&mut z.by_name("readme").expect("readme"), &mut text)
        .expect("read");
    assert_eq!(text, "new readme");
}

#[test]
fn add_to_an_archive_with_staged_changes_only_stages() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let sources = add_sources(p, 1);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("fresh").expect("f"))
        .expect("mkdir");
    let before = zip_names(&zip);
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": sources}),
    )
    .expect("add");
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    assert_eq!(zip_names(&zip), before, "the file is untouched");
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(mount.is_dirty(), "staged for the unsaved bar");
    poll_until(&mut s, "listing", |s| s.dirs.state(&root).is_ready());
    assert!(names(&s, &root).contains(&"f0.txt".to_string()));
}

#[test]
fn add_to_an_opened_archive_stages_for_the_unsaved_bar() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let sources = add_sources(p, 1);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let container = VfsUri::parse(zip.to_str().expect("utf8")).expect("uri");
    poll_until(&mut s, "tree", |s| s.archives.is_opened(&container));
    let before = zip_names(&zip);
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": sources}),
    )
    .expect("add");
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    assert_eq!(zip_names(&zip), before, "not saved behind the user's back");
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(mount.is_dirty());
    assert_eq!(fm_engine::archive_cmds::dirty_handles(&s).len(), 1);
}

#[test]
fn add_to_a_folder_inside_the_archive() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let sources = add_sources(p, 1);
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": sources, "dir": root.join("docs").expect("d").to_string()}),
    )
    .expect("add");
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    let docs = root.join("docs").expect("d");
    s.dispatch("nav.open", &json!(docs.to_string()))
        .expect("open docs");
    poll_until(&mut s, "listing", |s| s.dirs.state(&docs).is_ready());
    assert!(names(&s, &docs).contains(&"f0.txt".to_string()));
}

#[test]
fn add_with_bad_params_changes_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    std::fs::write(p.join("plain.txt"), b"x").expect("plain");
    std::fs::write(p.join("old.rar"), b"Rar!").expect("rar");
    let sources = add_sources(p, 1);
    let before = std::fs::read(&zip).expect("bytes");
    let mut s = Session::new().expect("session");
    let z = uri(&zip);
    for bad in [
        json!(null),
        json!({"uri": z, "sources": []}),
        json!({"uri": z, "sources": 5}),
        json!({"uri": uri(&p.join("plain.txt")), "sources": sources}),
        json!({"uri": uri(&p.join("old.rar")), "sources": sources}),
        json!({"uri": z, "sources": [z]}),
        json!({"uri": z, "sources": [uri(p)]}),
        json!({"uri": z, "sources": sources, "dir": uri(p)}),
        json!({"sources": sources}),
    ] {
        assert!(s.dispatch("archive.add", &bad).is_err(), "{bad}");
    }
    assert!(s.fileops.jobs().is_empty(), "nothing was started");
    assert_eq!(std::fs::read(&zip).expect("bytes"), before);
}

#[test]
fn add_of_ten_thousand_items_is_one_job() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    std::fs::create_dir_all(p.join("many")).expect("many");
    let mut sources = Vec::new();
    for i in 0..10_000 {
        let f = p.join(format!("many/f{i}"));
        std::fs::write(&f, b"x").expect("f");
        sources.push(uri(&f));
    }
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.add",
        &json!({"uri": uri(&zip), "sources": sources}),
    )
    .expect("add");
    assert_eq!(s.fileops.jobs().len(), 1);
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    assert_eq!(zip_names(&zip).len(), 3 + 10_000);
    sources_over_limit(&mut s, &zip);
}

fn sources_over_limit(s: &mut Session, zip: &Path) {
    let many: Vec<String> = (0..10_001).map(|i| format!("/tmp/x{i}")).collect();
    assert!(
        s.dispatch("archive.add", &json!({"uri": uri(zip), "sources": many}))
            .is_err()
    );
}

#[test]
fn dropping_a_selection_on_an_archive_file_adds_it_in_one_job() {
    use fm_engine::{DropAction, DropModifiers, DropOrigin, DropResolution};
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let sources = add_sources(p, 19);
    let mut s = Session::new().expect("session");
    let dest = VfsUri::parse(zip.to_str().expect("utf8")).expect("uri");
    let list: Vec<VfsUri> = sources
        .iter()
        .map(|u| VfsUri::parse(u).expect("uri"))
        .collect();
    // Always an Add, whatever the modifiers say about copying or moving.
    for mods in [
        DropModifiers::default(),
        DropModifiers {
            ctrl: true,
            shift: false,
        },
        DropModifiers {
            ctrl: false,
            shift: true,
        },
    ] {
        assert_eq!(
            s.preview_drop(&list, &dest, mods, DropOrigin::Internal),
            DropResolution::Do(DropAction::Add)
        );
    }
    // Not onto itself, and not both modifiers.
    assert!(matches!(
        s.preview_drop(
            std::slice::from_ref(&dest),
            &dest,
            DropModifiers::default(),
            DropOrigin::Internal
        ),
        DropResolution::NoDrop(_)
    ));
    s.dispatch(
        "dnd.drop",
        &json!({"sources": sources, "dest": dest.to_string()}),
    )
    .expect("drop");
    assert_eq!(s.fileops.jobs().len(), 1, "one job for the whole drop");
    assert!(wait_done(&mut s).iter().all(|st| *st == OpState::Finished));
    assert_eq!(zip_names(&zip).len(), 3 + 2 + 19);
    for src in &list {
        let local = src.local_path().expect("local");
        assert!(local.exists(), "dropping copies, never moves");
    }
    // A plain file that is not an archive is no add target.
    std::fs::write(p.join("note.txt"), b"x").expect("note");
    let note = VfsUri::parse(p.join("note.txt").to_str().expect("utf8")).expect("uri");
    assert!(!s.is_archive_drop_target(&note));
    assert!(s.is_archive_drop_target(&dest));
    std::fs::create_dir(p.join("plain")).expect("plain");
    assert!(
        s.dispatch(
            "dnd.drop",
            &json!({"sources": [uri(&p.join("src/f0.txt"))], "dest": uri(&p.join("plain")), "ctrl": true})
        )
        .is_ok(),
        "a folder drop still copies"
    );
}

#[test]
fn moving_out_of_a_writable_archive_copies_out_and_stages_the_removal() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = p.join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    std::fs::create_dir(p.join("out")).expect("out");
    let before = std::fs::read(&zip).expect("bytes");

    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    let docs = root.join("docs").expect("docs");
    s.dispatch(
        "file.move",
        &json!({"sources": [docs.to_string(), root.join("readme").expect("r").to_string()], "dest": uri(&p.join("out"))}),
    )
    .expect("move");
    wait_jobs(&mut s);

    assert_eq!(files_in(&p.join("out")), vec!["docs", "readme"]);
    assert_eq!(
        std::fs::read(p.join("out/docs/old.txt")).expect("moved"),
        b"old"
    );
    assert!(names(&s, &root).is_empty(), "removal is staged");
    assert_eq!(std::fs::read(&zip).expect("bytes"), before, "not saved yet");
    let mount = s.archives.for_uri(&root).cloned().expect("mount");
    assert!(mount.is_dirty());
}

#[test]
fn a_read_only_archive_offers_copy_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("a.txt", &[1u8; 300])]);
    let new = zip_with(&[("a.txt", &[2u8; 300])]);
    let size = old.len() / 2 + 1;
    for (i, chunk) in old.chunks(size).enumerate() {
        std::fs::write(p.join(format!(".set.zip.{:03}.fmbak", i + 1)), chunk).expect("bak");
    }
    write_parts(p, "set.zip", &new, size);
    std::fs::create_dir(p.join("out")).expect("out");

    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let a = root.join("a.txt").expect("a");
    let sources = json!([a.to_string()]);
    let err = s
        .dispatch(
            "file.move",
            &json!({"sources": sources, "dest": uri(&p.join("out"))}),
        )
        .expect_err("move refused");
    assert!(err.to_string().contains("Copy"), "{err}");
    assert!(files_in(&p.join("out")).is_empty(), "nothing was copied");

    let drop = s.preview_drop(
        std::slice::from_ref(&a),
        &VfsUri::parse(p.join("out").to_str().expect("utf8")).expect("out"),
        fm_engine::DropModifiers {
            ctrl: false,
            shift: true,
        },
        fm_engine::DropOrigin::Internal,
    );
    assert!(
        matches!(drop, fm_engine::DropResolution::NoDrop(_)),
        "{drop:?}"
    );

    s.dispatch(
        "file.copy",
        &json!({"sources": sources, "dest": uri(&p.join("out"))}),
    )
    .expect("copy");
    wait_jobs(&mut s);
    assert_eq!(files_in(&p.join("out")), vec!["a.txt"]);
}

fn big_bytes(len: usize) -> Vec<u8> {
    (0..len as u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect()
}

#[test]
fn join_concatenates_numbered_parts_and_keeps_them() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let whole = big_bytes(1000);
    let parts = write_parts(p, "dump.tar.gz", &whole, 300);
    assert_eq!(parts.len(), 4);
    let mut s = Session::new().expect("session");
    // Any part names the set.
    s.dispatch(
        "archive.join",
        &json!({"uri": uri(&p.join("dump.tar.gz.003"))}),
    )
    .expect("join");
    wait_jobs(&mut s);
    assert_eq!(std::fs::read(p.join("dump.tar.gz")).expect("joined"), whole);
    for part in &parts {
        assert!(p.join(part).exists(), "{part} is kept");
    }
    assert_eq!(files_in(p).len(), 5, "no temporary file is left");

    // The target exists now: the job asks, declining changes nothing.
    std::fs::write(p.join("dump.tar.gz"), b"mine").expect("mine");
    s.dispatch(
        "archive.join",
        &json!({"uri": uri(&p.join("dump.tar.gz.001"))}),
    )
    .expect("join again");
    poll_until(&mut s, "the question", |s| {
        s.fileops.pending_question().is_some()
    });
    let job = s.fileops.pending_question().expect("question");
    assert!(
        job.question
            .as_deref()
            .unwrap_or("")
            .contains("already exists"),
        "{:?}",
        job.question
    );
    let id = job.id;
    s.dispatch("job.answer", &json!({"job": id, "go": false}))
        .expect("no");
    wait_done(&mut s);
    assert_eq!(s.fileops.get(id).expect("job").state, OpState::Cancelled);
    assert_eq!(std::fs::read(p.join("dump.tar.gz")).expect("kept"), b"mine");

    // Yes replaces it, also at an explicit target.
    s.dispatch(
        "archive.join",
        &json!({"uri": uri(&p.join("dump.tar.gz.001"))}),
    )
    .expect("join third");
    poll_until(&mut s, "the question", |s| {
        s.fileops.pending_question().is_some()
    });
    let id = s.fileops.pending_question().expect("question").id;
    s.dispatch("job.answer", &json!({"job": id, "go": true}))
        .expect("yes");
    wait_done(&mut s);
    assert_eq!(std::fs::read(p.join("dump.tar.gz")).expect("joined"), whole);
    s.dispatch(
        "archive.join",
        &json!({"uri": uri(&p.join("dump.tar.gz.002")), "target": uri(&p.join("copy.bin"))}),
    )
    .expect("join to target");
    poll_until(&mut s, "done", |s| {
        s.fileops.jobs().iter().all(|j| !j.state.is_running())
    });
    assert_eq!(std::fs::read(p.join("copy.bin")).expect("copy"), whole);
}

#[test]
fn join_refuses_a_missing_part_and_bad_input() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    write_parts(p, "dump.7z", &big_bytes(1000), 300);
    std::fs::remove_file(p.join("dump.7z.003")).expect("rm");
    let mut s = Session::new().expect("session");
    s.dispatch("archive.join", &json!({"uri": uri(&p.join("dump.7z.001"))}))
        .expect("submitted");
    poll_until(&mut s, "failed", |s| {
        s.fileops.jobs().iter().all(|j| !j.state.is_running())
    });
    let job = s.fileops.jobs().last().expect("job");
    assert!(matches!(job.state, OpState::Failed(_)), "{:?}", job.state);
    assert!(
        format!("{:?}", job.state).contains("003"),
        "{:?}",
        job.state
    );
    assert!(!p.join("dump.7z").exists(), "nothing is written");
    assert_eq!(files_in(p).len(), 3);

    for bad in [
        json!({"uri": uri(&p.join("site.zip"))}),
        json!({"uri": uri(&p.join("dump.z01"))}),
        json!({"uri": uri(&p.join("data.part1.rar"))}),
        json!({"uri": 7}),
        json!({}),
        json!("x"),
    ] {
        assert!(s.dispatch("archive.join", &bad).is_err(), "{bad}");
    }
}

/// The workspace tab showing an archive with staged changes carries the unsaved marker,
/// also while another tab is active, until the changes are saved or discarded
/// (`archive-editing` "Unsaved archive changes are visible").
#[test]
fn the_tab_showing_a_changed_archive_is_marked_unsaved() {
    let dir = tempfile::tempdir().expect("tmp");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let mut s = Session::new().expect("session");
    let root = open_zip(&mut s, &zip);
    assert!(!s.tab_shows_unsaved_archive(0), "nothing staged yet");
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("new").expect("n"))
        .expect("mkdir");
    assert!(s.tab_shows_unsaved_archive(0));

    s.dispatch("workspace.new", &json!({})).expect("new tab");
    assert_eq!(s.workspaces.active, 1);
    assert!(
        !s.tab_shows_unsaved_archive(1),
        "the new tab shows no archive"
    );
    assert!(
        s.tab_shows_unsaved_archive(0),
        "the parked tab keeps its marker"
    );
    assert!(!s.tab_shows_unsaved_archive(7), "a tab that does not exist");

    s.dispatch(
        "archive.discard",
        &json!({"uri": root.to_string(), "confirm": true}),
    )
    .expect("discard");
    assert!(!s.tab_shows_unsaved_archive(0), "cleared by Discard");
}

/// Write commands are disabled inside a read-only archive, with the reason for the
/// tooltip; inside a zip they stay enabled (`archive-browsing` "Supported formats").
#[test]
fn write_commands_are_disabled_with_a_reason_inside_read_only_archives() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let gz = p.join("access.log.gz");
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(b"line\n").expect("gz");
    std::fs::write(&gz, enc.finish().expect("finish")).expect("write");
    std::fs::write(p.join("site.zip"), zip_bytes()).expect("zip");

    let mut s = Session::new().expect("session");
    let enabled = |s: &Session, id: &str| (s.registry.get(id).expect(id).enabled)(s);
    open_zip(&mut s, &p.join("site.zip"));
    assert_eq!(fm_engine::archive_cmds::focused_read_only(&s), None);
    assert!(enabled(&s, "file.mkdir"), "a zip takes new folders");

    open_zip(&mut s, &gz);
    assert_eq!(
        fm_engine::archive_cmds::focused_read_only(&s),
        Some("single compressed files are read-only")
    );
    s.dispatch("edit.select_all", &json!(null)).expect("select");
    for id in fm_engine::archive_cmds::WRITE_COMMANDS {
        assert!(!enabled(&s, id), "{id} is disabled in a .gz");
    }
    assert!(enabled(&s, "file.clip_copy"), "copying out still works");
}

/// The ninth archive inside archives is not opened: its pane shows the nesting-limit
/// state instead of a listing (`archive-browsing` "Nesting limit").
#[test]
fn an_archive_nested_too_deep_shows_the_limit_in_its_pane() {
    let dir = tempfile::tempdir().expect("tmp");
    let levels = fm_engine::archive_cmds::MAX_NESTING + 1;
    let mut bytes = zip_with(&[("bottom.txt", b"deep")]);
    for _ in 1..levels {
        bytes = zip_with(&[("n.zip", &bytes)]);
    }
    let outer = dir.path().join("n.zip");
    std::fs::write(&outer, &bytes).expect("zip");
    let mut s = Session::new().expect("session");
    let mut root = open_zip(&mut s, &outer);
    for _ in 2..levels {
        let inner = root.join("n.zip").expect("inner");
        s.dispatch("archive.enter", &json!({"uri": inner.to_string()}))
            .expect("enter");
        root = s
            .layout
            .state(s.layout.focused())
            .expect("pane")
            .location
            .clone();
        poll_until(&mut s, "listing", |s| s.dirs.state(&root).is_ready());
    }
    let inner = root.join("n.zip").expect("inner");
    s.dispatch("archive.enter", &json!({"uri": inner.to_string()}))
        .expect("the ninth level opens a pane state, not an error");
    let deepest = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    assert_eq!(fm_engine::archive_cmds::nesting(&deepest), levels);
    poll_until(&mut s, "failure", |s| {
        matches!(s.dirs.state(&deepest), fm_engine::DirState::Failed { .. })
    });
    let fm_engine::DirState::Failed { message, .. } = s.dirs.state(&deepest) else {
        panic!("expected a failed listing");
    };
    assert_eq!(
        fm_engine::archive_cmds::problem(&deepest, &message),
        Some(fm_engine::archive_cmds::ArchiveProblem::TooDeep),
        "{message}"
    );
    s.dispatch("nav.up", &json!(null)).expect("up");
    let back = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    assert_eq!(
        back, root,
        "Show the archive file goes to the folder holding it"
    );
}

/// The archive file's context menu offers File › Archive for that file's opened archive:
/// any part of a split set finds it, the menu rules match, and its commands act on it from
/// the folder the set is in.
#[test]
fn an_archive_file_finds_its_opened_archive_for_the_context_flyout() {
    use fm_engine::archive_edit_cmds::{FLYOUT, enabled_on};
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let old = zip_with(&[("a.txt", &[7u8; 300])]);
    write_parts(p, "set.zip", &old, old.len() / 2 + 1);
    std::fs::write(p.join("other.zip"), zip_with(&[("b.txt", b"b")])).expect("other");
    let mut s = Session::new().expect("session");
    let part = |n: &str| VfsUri::parse(&uri(&p.join(n))).expect("uri");
    assert!(
        s.archives.of_file(&part("set.zip.002")).is_none(),
        "not opened yet"
    );
    let root = open_zip(&mut s, &p.join("set.zip.001"));
    let vfs = s.backends.require(&root).expect("backend");
    vfs.create_dir(&root.join("mine").expect("m"))
        .expect("mkdir");
    // Back in the folder: the pane is no longer inside the archive.
    s.dispatch("nav.open", &json!(uri(p))).expect("up");
    let mount = s
        .archives
        .of_file(&part("set.zip.002"))
        .cloned()
        .expect("found from another part");
    assert_eq!(mount.root(), &root);
    assert!(s.archives.of_file(&part("other.zip")).is_none());
    for id in [
        "archive.save",
        "archive.discard",
        "archive.changes",
        "archive.save_as",
    ] {
        assert!(enabled_on(id, &mount), "{id}");
    }
    assert!(
        !enabled_on("archive.password.remove", &mount),
        "not encrypted"
    );
    assert!(
        FLYOUT
            .iter()
            .flatten()
            .all(|id| s.registry.get(id).is_some())
    );
    s.dispatch("archive.save", &json!({"uri": root.to_string()}))
        .expect("save from the folder");
    assert_eq!(wait_done(&mut s).last(), Some(&OpState::Finished));
    assert!(!mount.is_dirty());
}

/// Close Archive on a changed archive: refused as is (the UI asks first), `unsaved:
/// "discard"` drops the changes and closes, `unsaved: "save"` saves and closes once the
/// save job finished; a bad choice is an error.
#[test]
fn closing_a_changed_archive_saves_or_discards_first() {
    let opened =
        |s: &Session, root: &VfsUri| s.archives.for_uri(root).and_then(|m| m.opened()).is_some();
    for choice in ["discard", "save"] {
        let dir = tempfile::tempdir().expect("tmp");
        let zip = dir.path().join("site.zip");
        std::fs::write(&zip, zip_with(&[("a.txt", b"a")])).expect("zip");
        let mut s = Session::new().expect("session");
        let root = open_zip(&mut s, &zip);
        let vfs = s.backends.require(&root).expect("backend");
        vfs.create_dir(&root.join("mine").expect("m"))
            .expect("mkdir");
        let file = VfsUri::parse(&uri(&zip)).expect("file");
        let (found, name, count) =
            fm_engine::archive_cmds::unsaved_for_close(&s, &file).expect("unsaved");
        assert_eq!((found, name.as_str(), count), (root.clone(), "site.zip", 1));
        assert!(
            s.dispatch("archive.unmount", &json!({"uri": root.to_string()}))
                .is_err(),
            "closing as is would lose the change"
        );
        assert!(
            s.dispatch(
                "archive.unmount",
                &json!({"uri": root.to_string(), "unsaved": 5})
            )
            .is_err()
        );
        s.dispatch(
            "archive.unmount",
            &json!({"uri": root.to_string(), "unsaved": choice}),
        )
        .expect("close");
        if choice == "save" {
            assert_eq!(wait_done(&mut s).last(), Some(&OpState::Finished));
            poll_until(&mut s, "closed after the save", |s| !opened(s, &root));
            assert_eq!(zip_names(&zip), vec!["a.txt", "mine/"], "saved");
        } else {
            assert!(!opened(&s, &root), "closed at once");
            assert_eq!(zip_names(&zip), vec!["a.txt"], "the change is gone");
        }
        assert!(fm_engine::archive_cmds::unsaved_for_close(&s, &file).is_none());
    }
}
