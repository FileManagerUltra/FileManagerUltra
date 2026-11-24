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

#[test]
fn read_only_archives_refuse_changes() {
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path();
    let zip = zip_bytes();
    let half = zip.len() / 2;
    std::fs::write(p.join("set.zip.001"), &zip[..half]).expect("p1");
    std::fs::write(p.join("set.zip.002"), &zip[half..]).expect("p2");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "archive.enter",
        &json!({"uri": uri(&p.join("set.zip.001"))}),
    )
    .expect("enter");
    let root = s
        .layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone();
    poll_until(&mut s, "listing", |s| s.dirs.state(&root).is_ready());
    let vfs = s.backends.require(&root).expect("backend");
    let err = vfs
        .create_dir(&root.join("x").expect("x"))
        .expect_err("read-only");
    assert!(err.to_string().contains("read-only"), "{err}");
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
