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
