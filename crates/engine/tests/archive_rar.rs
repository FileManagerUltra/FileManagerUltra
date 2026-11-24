//! RAR through an installed tool (add-archive-vfs 4.6), driven with a stand-in `7z` script
//! (Unix only): entering lists it, files read through the tool, any volume opens the set,
//! and a missing volume is named before the tool runs. Own process: the tool choice is
//! process-wide.
#![cfg(unix)]
#![allow(clippy::expect_used, clippy::panic)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::Session;
use fm_vfs::VfsUri;

const LISTING: &str = "Path = docs\nFolder = +\nSize = 0\nModified = 2024-01-02 03:04:05\n\nPath = docs/a.txt\nFolder = -\nSize = 4\nPacked Size = 9\nModified = 2024-01-02 03:04:05\nEncrypted = -\n";

fn fake_7z(dir: &Path) -> String {
    let script = dir.join("7z");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nd=$(dirname \"$0\")\nprintf '%s\\n' \"$@\" >> \"$d/calls.txt\"\ncase \"$1\" in\n  l) printf '{}';;\n  e) printf 'data';;\nesac\n",
            LISTING.replace('\n', "\\n")
        ),
    )
    .expect("script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script.to_string_lossy().into_owned()
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

fn enter(s: &mut Session, file: &Path) -> VfsUri {
    s.dispatch(
        "archive.enter",
        &json!({"uri": file.to_str().expect("utf8")}),
    )
    .expect("enter");
    s.layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone()
}

#[test]
fn rar_archives_and_volumes_open_through_the_tool() {
    let tools = tempfile::tempdir().expect("tools");
    let work = tempfile::tempdir().expect("work");
    let tool = fake_7z(tools.path());
    let mut s = Session::new().expect("session");
    s.dispatch(
        "prefs.set",
        &json!({"key": "archives_rar_tool", "value": tool}),
    )
    .expect("choose the tool");
    assert!(fm_engine::archive_cmds::is_archive_name("photos.rar"));

    std::fs::write(work.path().join("photos.rar"), b"Rar!\x1a\x07\x01\x00").expect("rar");
    let root = enter(&mut s, &work.path().join("photos.rar"));
    let docs = root.join("docs").expect("docs");
    poll_until(&mut s, "the listing", |s| s.dirs.state(&root).is_ready());
    let file = docs.join("a.txt").expect("a");
    let vfs = s.backends.require(&file).expect("backend");
    let mut text = String::new();
    std::io::Read::read_to_string(&mut vfs.open_read(&file).expect("open"), &mut text)
        .expect("read");
    assert_eq!(text, "data");
    let calls = std::fs::read_to_string(tools.path().join("calls.txt")).expect("calls");
    assert!(
        calls.contains("-spd") && calls.contains("docs/a.txt"),
        "{calls}"
    );

    // Volumes: any one opens the set at its first volume.
    for v in ["set.part1.rar", "set.part2.rar", "set.part3.rar"] {
        std::fs::write(work.path().join(v), b"Rar!\x1a\x07\x01\x00").expect("volume");
    }
    let from_second = enter(&mut s, &work.path().join("set.part2.rar"));
    let from_first = enter(&mut s, &work.path().join("set.part1.rar"));
    assert_eq!(from_second, from_first);
    poll_until(&mut s, "the set's listing", |s| {
        s.dirs.state(&from_first).is_ready()
    });
    let shown = fm_engine::item_cmds::display_path(&from_first);
    assert!(shown.ends_with("set.part1.rar"), "{shown}");

    // A missing volume is named and the tool is not asked.
    std::fs::remove_file(work.path().join("set.part2.rar")).expect("rm");
    s.dispatch(
        "archive.unmount",
        &json!({"uri": work.path().join("set.part1.rar").to_str()}),
    )
    .expect("close");
    std::fs::write(tools.path().join("calls.txt"), b"").expect("reset");
    let root = enter(&mut s, &work.path().join("set.part1.rar"));
    poll_until(&mut s, "the failed listing", |s| {
        matches!(s.dirs.state(&root), fm_engine::DirState::Failed { .. })
    });
    let fm_engine::DirState::Failed { message, .. } = s.dirs.state(&root) else {
        panic!("not failed")
    };
    assert!(
        message.contains("Part 2 of set.rar is missing (set.part2.rar)"),
        "{message}"
    );
    let calls = std::fs::read_to_string(tools.path().join("calls.txt")).expect("calls");
    assert!(calls.is_empty(), "the tool was not run: {calls}");
}
