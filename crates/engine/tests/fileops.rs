//! File jobs through `Session`: commands in, polled events out, never blocking the caller.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::{DirState, FileOps, JobId, OpState, Params, Session};
use fm_testkit::{FaultFs, MemFs};
use fm_vfs::{FmError, Vfs, VfsUri};

fn uri(s: &str) -> VfsUri {
    VfsUri::parse(s).expect("uri")
}

fn session_on(fs: Arc<dyn fm_vfs::Vfs>) -> Session {
    let mut s = Session::new().expect("session");
    s.backends.register(fs);
    s
}

fn poll_until(s: &mut Session, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        if done(s) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn only_job(ops: &FileOps) -> JobId {
    ops.jobs().last().expect("a job").id
}

fn finish(s: &mut Session, id: JobId) -> OpState {
    poll_until(s, "job to finish", |s| {
        s.fileops.get(id).is_some_and(|j| !j.state.is_running())
    });
    s.fileops.get(id).expect("job").state.clone()
}

fn dispatch(s: &mut Session, id: &str, params: Params) -> Result<(), FmError> {
    s.dispatch(id, &params)
}

#[test]
fn copy_runs_off_thread_records_undo_and_refreshes_the_destination() {
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"hello").expect("put");
    fs.mkdirs_str("/dst").expect("dst");
    let mut s = session_on(Arc::new(fs.clone()));
    s.ensure_read(uri("/dst")).expect("read");
    poll_until(&mut s, "dst listing", |s| {
        s.dirs.state(&uri("/dst")).is_ready()
    });
    assert!(s.dirs.entries(&uri("/dst")).is_some_and(|e| e.is_empty()));

    dispatch(
        &mut s,
        "file.copy",
        json!({"sources": ["/src/a.txt"], "dest": "/dst"}),
    )
    .expect("dispatch");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/dst/a.txt").expect("copied"), b"hello");
    assert_eq!(s.undo.len(), 1);

    poll_until(&mut s, "dst refresh", |s| {
        s.dirs
            .entries(&uri("/dst"))
            .is_some_and(|e| e.iter().any(|x| x.name == "a.txt"))
    });
}

#[test]
fn a_conflict_parks_the_job_until_resolved() {
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"new").expect("put");
    fs.put_str("/dst/a.txt", b"old").expect("put");
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(
        &mut s,
        "file.copy",
        json!({"sources": ["/src/a.txt"], "dest": "/dst"}),
    )
    .expect("dispatch");
    let id = only_job(&s.fileops);
    poll_until(&mut s, "conflict", |s| {
        s.fileops.pending_conflict().is_some()
    });
    assert!(s.command_enabled("job.resolve_conflict"));
    assert_eq!(fs.get_str("/dst/a.txt").expect("old"), b"old");

    dispatch(
        &mut s,
        "job.resolve_conflict",
        json!({"job": id, "choice": "overwrite"}),
    )
    .expect("resolve");
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/dst/a.txt").expect("new"), b"new");
    // Answering twice is an error, not a hang.
    assert!(
        dispatch(
            &mut s,
            "job.resolve_conflict",
            json!({"job": id, "choice": "skip"})
        )
        .is_err()
    );
}

#[test]
fn cancelling_a_parked_job_frees_the_worker() {
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"new").expect("put");
    fs.put_str("/dst/a.txt", b"old").expect("put");
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(
        &mut s,
        "file.copy",
        json!({"sources": ["/src/a.txt"], "dest": "/dst"}),
    )
    .expect("dispatch");
    let id = only_job(&s.fileops);
    poll_until(&mut s, "conflict", |s| {
        s.fileops.pending_conflict().is_some()
    });
    dispatch(&mut s, "job.cancel", json!({"job": id})).expect("cancel");
    assert_eq!(finish(&mut s, id), OpState::Cancelled);
    assert_eq!(fs.get_str("/dst/a.txt").expect("untouched"), b"old");
    assert!(
        dispatch(&mut s, "job.cancel", json!({"job": 9999})).is_err(),
        "unknown job"
    );
}

#[test]
fn poll_never_blocks_and_progress_is_monotonic() {
    let mem = MemFs::new();
    mem.put_str("/src/big.bin", &[7u8; 256]).expect("put");
    mem.mkdirs_str("/dst").expect("dst");
    let fs = FaultFs::new(mem);
    {
        let mut faults = fs.faults();
        faults.read_chunk = Some(8);
        faults.read_hook = Some(Arc::new(|| std::thread::sleep(Duration::from_millis(8))));
    }
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(
        &mut s,
        "file.copy",
        json!({"sources": ["/src/big.bin"], "dest": "/dst"}),
    )
    .expect("dispatch");
    let id = only_job(&s.fileops);

    let mut last = 0u64;
    let mut slowest = Duration::ZERO;
    let mut saw_running = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let t = Instant::now();
        s.poll();
        slowest = slowest.max(t.elapsed());
        let job = s.fileops.get(id).expect("job");
        assert!(job.progress.bytes.done >= last, "progress went backwards");
        last = job.progress.bytes.done;
        if job.state.is_running() {
            saw_running = true;
        } else {
            break;
        }
        assert!(Instant::now() < deadline, "copy never finished");
        std::thread::sleep(Duration::from_millis(3));
    }
    assert!(saw_running, "job finished before the first poll");
    assert!(
        slowest < Duration::from_millis(100),
        "poll blocked: {slowest:?}"
    );
    assert_eq!(last, 256);
    assert_eq!(fs.inner().get_str("/dst/big.bin").expect("dst").len(), 256);
}

#[test]
fn rename_validates_synchronously_and_undo_restores_the_name() {
    let fs = MemFs::new();
    fs.put_str("/d/a.txt", b"x").expect("put");
    let mut s = session_on(Arc::new(fs.clone()));

    let err = dispatch(
        &mut s,
        "file.rename",
        json!({"uri": "/d/a.txt", "name": "x/y"}),
    )
    .expect_err("slash");
    assert!(matches!(err, FmError::InvalidParams(_)));
    assert!(
        s.fileops.jobs().is_empty(),
        "bad params must not start a job"
    );

    dispatch(
        &mut s,
        "file.rename",
        json!({"uri": "/d/a.txt", "name": "b.txt"}),
    )
    .expect("rename");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert!(fs.exists(&uri("/d/b.txt")));

    assert!(s.command_enabled("edit.undo"));
    dispatch(&mut s, "edit.undo", Params::Null).expect("undo");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert!(fs.exists(&uri("/d/a.txt")));
    assert!(!fs.exists(&uri("/d/b.txt")));
    assert!(!s.command_enabled("edit.undo"), "the record was consumed");
}

#[test]
fn a_stale_undo_fails_actionably_and_is_dropped() {
    let fs = MemFs::new();
    fs.put_str("/d/a.txt", b"x").expect("put");
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(
        &mut s,
        "file.rename",
        json!({"uri": "/d/a.txt", "name": "b.txt"}),
    )
    .expect("rename");
    let id = only_job(&s.fileops);
    finish(&mut s, id);
    fs.put_str("/d/a.txt", b"occupied").expect("occupy");

    dispatch(&mut s, "edit.undo", Params::Null).expect("undo");
    let id = only_job(&s.fileops);
    assert!(matches!(finish(&mut s, id), OpState::Failed(_)));
    assert_eq!(fs.get_str("/d/b.txt").expect("kept"), b"x");
    assert!(s.undo.is_empty());
}

#[test]
fn newfile_creates_empty_file_and_undo_removes_it() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d").expect("d");
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(
        &mut s,
        "file.newfile",
        json!({"parent": "/d", "name": "n.txt"}),
    )
    .expect("newfile");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/d/n.txt").expect("file"), b"");
    assert_eq!(s.undo.len(), 1);
    assert!(dispatch(&mut s, "file.newfile", json!({"parent": "/d"})).is_err());
    assert!(
        dispatch(
            &mut s,
            "file.newfile",
            json!({"parent": "/d", "name": "../x"})
        )
        .is_err()
    );
}

#[test]
fn tree_directory_is_the_target_until_the_listing_is_used() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d/sub").expect("sub");
    let mut s = session_on(Arc::new(fs.clone()));
    {
        let pane = s.layout.state_mut(1).expect("pane");
        pane.location = uri("/d");
        pane.tree.focused = Some(uri("/d/sub"));
        pane.tree.active = true;
    }
    assert!(s.command_enabled("file.delete"));
    assert_eq!(
        fm_engine::file_cmds::focused_selection(&s),
        vec![uri("/d/sub")]
    );
    s.layout.state_mut(1).expect("pane").tree.active = false;
    assert!(!s.command_enabled("file.delete"));
    let pane = s.layout.state_mut(1).expect("pane");
    pane.tree.focused = Some(uri("/"));
    pane.tree.active = true;
    assert!(!s.command_enabled("file.delete"), "roots are never targets");
}

#[test]
fn copy_retargets_inside_links_only_when_the_preference_is_on() {
    for (retarget, expected) in [(true, "/dst/p/lib/a"), (false, "/src/p/lib/a")] {
        let fs = MemFs::new();
        fs.put_str("/src/p/lib/a", b"x").expect("put");
        fs.put_symlink(&uri("/src/p/link"), "/src/p/lib/a")
            .expect("link");
        fs.mkdirs_str("/dst").expect("dst");
        let mut s = session_on(Arc::new(fs.clone()));
        dispatch(&mut s, "prefs.links.retarget", json!({"value": retarget})).expect("pref");
        dispatch(
            &mut s,
            "file.copy",
            json!({"sources": ["/src/p"], "dest": "/dst"}),
        )
        .expect("copy");
        let id = only_job(&s.fileops);
        assert_eq!(finish(&mut s, id), OpState::Finished);
        match fs.stat(&uri("/dst/p/link")).expect("stat").kind {
            fm_vfs::EntryKind::Symlink { target } => assert_eq!(target, expected),
            other => panic!("expected a link, got {other:?}"),
        }
    }
}

#[test]
fn mkdir_and_permanent_delete() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d").expect("d");
    let mut s = session_on(Arc::new(fs.clone()));
    dispatch(&mut s, "file.mkdir", json!({"parent": "/d", "name": "new"})).expect("mkdir");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert!(fs.exists(&uri("/d/new")));
    assert_eq!(s.undo.len(), 1);

    fs.put_str("/d/gone/inner.txt", b"x").expect("put");
    let unconfirmed = dispatch(
        &mut s,
        "file.delete_permanent",
        json!({"sources": ["/d/gone"]}),
    );
    assert!(unconfirmed.is_err());
    assert!(fs.exists(&uri("/d/gone/inner.txt")), "nothing removed");

    let before = s.undo.len();
    dispatch(
        &mut s,
        "file.delete_permanent",
        json!({"sources": ["/d/gone"], "confirm": true}),
    )
    .expect("delete");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert!(!fs.exists(&uri("/d/gone")));
    assert_eq!(s.undo.len(), before, "permanent delete is not undoable");
}

#[test]
fn deleting_the_shown_folder_moves_the_pane_to_the_highlighted_or_parent_folder() {
    let fs = MemFs::new();
    for book in ["Alastair", "Douglas", "Kevin/Part 1"] {
        fs.put_str(&format!("/books/{book}/01.mp3"), b"x")
            .expect("file");
    }
    let mut s = session_on(Arc::new(fs.clone()));
    let pane = s.layout.focused();
    let delete = |s: &mut Session, what: &str| {
        dispatch(
            s,
            "file.delete_permanent",
            json!({"sources": [what], "confirm": true}),
        )
        .expect("delete");
        let id = only_job(&s.fileops);
        assert_eq!(finish(s, id), OpState::Finished);
    };
    let location = |s: &Session| s.layout.state(pane).expect("pane").location.clone();

    // Deleted from the tree: its highlight has moved on to the next folder.
    dispatch(&mut s, "nav.open", json!({"uri": "/books/Alastair"})).expect("open");
    s.layout.state_mut(pane).expect("pane").tree.focused = Some(uri("/books/Douglas"));
    delete(&mut s, "/books/Alastair");
    assert_eq!(
        location(&s),
        uri("/books/Douglas"),
        "the highlighted folder is shown"
    );
    poll_until(&mut s, "Douglas to be read", |s| {
        s.dirs.state(&uri("/books/Douglas")).is_ready()
    });

    // A folder above the shown one goes, and the tree has nothing else highlighted.
    dispatch(&mut s, "nav.open", json!({"uri": "/books/Kevin/Part 1"})).expect("open");
    delete(&mut s, "/books/Kevin");
    assert_eq!(location(&s), uri("/books"), "the nearest surviving parent");

    // Deleting something elsewhere leaves the pane alone.
    fs.put_str("/books/Other/01.mp3", b"x").expect("file");
    delete(&mut s, "/books/Other");
    assert_eq!(location(&s), uri("/books"));
}

#[test]
fn drop_resolves_in_the_engine() {
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"a").expect("put");
    fs.put_str("/src/b.txt", b"b").expect("put");
    fs.mkdirs_str("/dst").expect("dst");
    let mut s = session_on(Arc::new(fs.clone()));

    // Shift forces a move.
    dispatch(
        &mut s,
        "dnd.drop",
        json!({"sources": ["/src/a.txt"], "dest": "/dst", "shift": true}),
    )
    .expect("drop");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert!(fs.exists(&uri("/dst/a.txt")) && !fs.exists(&uri("/src/a.txt")));

    // Ctrl+Shift and dropping onto the source folder are rejected without a job.
    let jobs = s.fileops.jobs().len();
    assert!(
        dispatch(
            &mut s,
            "dnd.drop",
            json!({"sources": ["/src/b.txt"], "dest": "/dst", "ctrl": true, "shift": true})
        )
        .is_err()
    );
    assert!(
        dispatch(
            &mut s,
            "dnd.drop",
            json!({"sources": ["/src/b.txt"], "dest": "/src"})
        )
        .is_err()
    );
    assert_eq!(s.fileops.jobs().len(), jobs);

    // External drop: the hostile path is reported, the valid one still imports.
    let err = dispatch(
        &mut s,
        "dnd.drop",
        json!({"origin": "external", "dest": "/dst", "sources": ["/src/b.txt", "not\u{0}a path"]}),
    )
    .expect_err("one rejected");
    assert!(matches!(err, FmError::InvalidParams(_)));
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/dst/b.txt").expect("imported"), b"b");
    assert!(fs.exists(&uri("/src/b.txt")), "external drops copy");
}

#[test]
fn copy_with_nothing_selected_is_a_noop_and_disabled() {
    let mut s = Session::new().expect("session");
    assert!(!s.command_enabled("file.copy"));
    dispatch(&mut s, "file.copy", Params::Null).expect("no-op");
    assert!(s.fileops.jobs().is_empty());
}

#[test]
fn finished_jobs_can_be_dismissed_but_running_ones_stay() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d").expect("d");
    let mut s = session_on(Arc::new(fs));
    dispatch(&mut s, "file.mkdir", json!({"parent": "/d", "name": "x"})).expect("mkdir");
    let id = only_job(&s.fileops);
    finish(&mut s, id);
    dispatch(&mut s, "job.dismiss", json!({"job": id})).expect("dismiss");
    assert!(s.fileops.jobs().is_empty());
    assert!(dispatch(&mut s, "job.dismiss", json!({"job": id})).is_err());
}

#[test]
fn fresh_session_has_no_jobs_or_undo() {
    let s = Session::new().expect("session");
    assert!(s.fileops.jobs().is_empty());
    assert!(s.undo.is_empty());
    assert!(matches!(s.dirs.state(&uri("/nope")), DirState::Absent));
}

#[test]
fn traversal_names_are_rejected_and_nothing_is_created() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d/sub").expect("mkdirs");
    fs.put_str("/d/f.txt", b"x").expect("put");
    let mut s = session_on(Arc::new(fs.clone()));
    for bad in ["../evil", "a/b", "..", ".", "", "a\\b", "nul\0"] {
        let params = json!({"name": bad, "parent": "/d"});
        assert!(
            dispatch(&mut s, "file.mkdir", params).is_err(),
            "mkdir {bad:?}"
        );
        let params = json!({"name": bad, "uri": "/d/f.txt"});
        assert!(
            dispatch(&mut s, "file.rename", params).is_err(),
            "rename {bad:?}"
        );
    }
    assert!(s.fileops.jobs().is_empty(), "no job may be started");
    assert!(fs.stat(&uri("/evil")).is_err());
    assert!(fs.stat(&uri("/d/f.txt")).is_ok());
}

#[test]
fn credentials_never_surface_in_errors_or_job_reports() {
    let mut s = session_on(Arc::new(MemFs::new()));
    let src = "sftp://user:hunter2@example.invalid/a";
    let dest = "sftp://user:hunter2@example.invalid/b";
    let mut text = String::new();
    for (id, params) in [
        ("dnd.drop", json!({"sources": [src], "dest": dest})),
        ("file.copy", json!({"sources": [src], "dest": dest})),
        (
            "file.delete_permanent",
            json!({"sources": [src], "confirm": true}),
        ),
    ] {
        if let Err(err) = dispatch(&mut s, id, params) {
            text.push_str(&err.to_string());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.fileops.running() > 0 && Instant::now() < deadline {
        s.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    for job in s.fileops.jobs() {
        text.push_str(&format!("{} {:?}", job.label, job.state));
        for (item, err) in job.summary.iter().flat_map(|x| x.failed.iter()) {
            text.push_str(&format!("{item} {err}"));
        }
    }
    assert!(!text.contains("hunter2"), "credential leaked: {text}");
}

#[test]
fn a_focused_thumbnail_grid_drives_the_file_commands() {
    use fm_engine::file_cmds::{GridSelection, focused_dir, focused_selection, other_pane_dir};
    let mut s = session_on(Arc::new(MemFs::new()));
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let panes = s.layout.panes();
    let (files, grid) = (panes[0], panes[1]);
    s.layout.state_mut(files).expect("pane").location = uri("/d");
    s.layout.focus_pane(grid).expect("focus");
    s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
        .expect("kind");
    s.grid_selection = Some(GridSelection {
        pane: grid,
        dir: uri("/d/pics"),
        items: vec![uri("/d/pics/a.png")],
    });
    assert_eq!(focused_selection(&s), vec![uri("/d/pics/a.png")]);
    assert_eq!(focused_dir(&s), Some(uri("/d/pics")));
    assert!(s.command_enabled("file.delete"));
    assert!(s.command_enabled("file.mkdir"));
    assert_eq!(
        other_pane_dir(&s),
        None,
        "the grid's own source is not a destination"
    );
    s.grid_selection = None;
    assert!(focused_selection(&s).is_empty());
}

#[test]
fn clipboard_copy_cut_and_paste_run_the_pipelines() {
    use fm_engine::file_cmds::GridSelection;
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"hello").expect("put");
    fs.put_str("/src/b.txt", b"bye").expect("put");
    fs.mkdirs_str("/dst").expect("dst");
    let mut s = session_on(Arc::new(fs.clone()));
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let grid = s.layout.panes()[1];
    s.layout.focus_pane(grid).expect("focus");
    s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
        .expect("kind");
    let select = |s: &mut Session, name: &str, dir: &str| {
        s.grid_selection = Some(GridSelection {
            pane: grid,
            dir: uri(dir),
            items: vec![uri(&format!("/src/{name}"))],
        });
    };

    select(&mut s, "a.txt", "/src");
    assert!(!s.command_enabled("file.clip_paste"), "nothing copied yet");
    dispatch(&mut s, "file.clip_copy", Params::Null).expect("copy");
    assert_eq!(s.info.as_ref().map(|i| i.0.as_str()), Some("Copied 1 item"));
    assert_eq!(
        s.clipboard_outbox,
        fm_vfs::native_path("/src/a.txt", cfg!(windows)),
        "the OS clipboard gets the native path"
    );
    select(&mut s, "a.txt", "/dst");
    let os_text = fm_vfs::native_path("/src/a.txt", cfg!(windows)).expect("native");
    dispatch(&mut s, "file.clip_paste", json!({"os_text": os_text})).expect("paste");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/dst/a.txt").expect("copied"), b"hello");
    assert!(fs.get_str("/src/a.txt").is_ok(), "copy keeps the source");

    // Pasting back into the source folder duplicates under a numbered name.
    select(&mut s, "a.txt", "/src");
    dispatch(&mut s, "file.clip_paste", Params::Null).expect("paste in place");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/src/a (2).txt").expect("duplicate"), b"hello");

    select(&mut s, "b.txt", "/src");
    dispatch(&mut s, "file.clip_cut", Params::Null).expect("cut");
    select(&mut s, "b.txt", "/dst");
    dispatch(&mut s, "file.clip_paste", Params::Null).expect("paste");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/dst/b.txt").expect("moved"), b"bye");
    assert!(fs.get_str("/src/b.txt").is_err(), "cut removes the source");
    assert!(s.clipboard.is_none(), "a cut is pasted once");
}

#[test]
fn paste_refuses_when_the_os_clipboard_changed_or_params_are_bad() {
    use fm_engine::file_cmds::GridSelection;
    let mut s = session_on(Arc::new(MemFs::new()));
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let grid = s.layout.panes()[1];
    s.layout.focus_pane(grid).expect("focus");
    s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
        .expect("kind");
    s.grid_selection = Some(GridSelection {
        pane: grid,
        dir: uri("/d"),
        items: vec![uri("/d/a")],
    });
    dispatch(&mut s, "file.clip_copy", Params::Null).expect("copy");
    assert!(
        dispatch(
            &mut s,
            "file.clip_paste",
            json!({"os_text": "something else"})
        )
        .is_err()
    );
    assert!(s.clipboard.is_none());
    assert!(dispatch(&mut s, "file.clip_paste", json!({"os_text": 5})).is_err());
    assert!(dispatch(&mut s, "file.clip_paste", json!({"dest": 5})).is_err());
    assert!(dispatch(&mut s, "file.clip_paste", json!({"dest": ""})).is_err());
}

#[test]
fn folder_menu_paste_uses_explicit_destination_not_listing_location() {
    let fs = MemFs::new();
    fs.put_str("/src/a.txt", b"hello").expect("file");
    fs.mkdirs_str("/target").expect("target");
    let mut s = session_on(Arc::new(fs.clone()));
    let pane = s.layout.state_mut(s.layout.focused()).expect("pane");
    pane.location = uri("/src");
    pane.listing.select_only(Some("a.txt"), 0);
    dispatch(&mut s, "file.clip_copy", Params::Null).expect("copy");
    dispatch(&mut s, "file.clip_paste", json!({"dest": "/target"})).expect("paste");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    assert_eq!(fs.get_str("/target/a.txt").expect("copied"), b"hello");
    assert_eq!(
        s.layout.state(s.layout.focused()).expect("pane").location,
        uri("/src")
    );
    assert!(fs.get_str("/src/a (2).txt").is_err());
}

#[test]
fn opening_a_remote_file_shows_up_as_a_job() {
    let fs = MemFs::new();
    fs.put_str("/r/big.bin", b"x").expect("put");
    let mut s = session_on(Arc::new(fs));
    let id = s
        .submit_op(fm_engine::OpRequest::OpenRemote {
            uri: uri("/r/big.bin"),
            name: "big.bin".to_string(),
            size: Some(u64::MAX),
        })
        .expect("submit");
    assert_eq!(s.fileops.get(id).expect("job").label, "Open big.bin");
    assert!(matches!(finish(&mut s, id), OpState::Failed(m) if m.contains("too large")));
}

#[test]
fn visible_local_folders_pick_up_files_added_behind_the_apps_back() {
    let fs = MemFs::new();
    fs.mkdirs_str("/watched").expect("dir");
    let mut s = session_on(Arc::new(fs.clone()));
    s.dispatch("nav.open", &Params::String("file:///watched".to_string()))
        .expect("open");
    poll_until(&mut s, "first listing", |s| {
        s.dirs.state(&uri("/watched")).is_ready()
    });
    fs.put_str("/watched/new.txt", b"hi").expect("put");
    poll_until(&mut s, "periodic re-read", |s| {
        s.dirs
            .entries(&uri("/watched"))
            .is_some_and(|e| e.iter().any(|x| x.name == "new.txt"))
    });
}

#[test]
fn mkdir_shows_up_in_a_cached_directory_listing() {
    let fs = MemFs::new();
    fs.mkdirs_str("/d/old").expect("d");
    let mut s = session_on(Arc::new(fs.clone()));
    s.ensure_read(uri("/d")).expect("read");
    poll_until(&mut s, "listing", |s| s.dirs.state(&uri("/d")).is_ready());
    dispatch(&mut s, "file.mkdir", json!({"parent": "/d", "name": "new"})).expect("mkdir");
    let id = only_job(&s.fileops);
    assert_eq!(finish(&mut s, id), OpState::Finished);
    poll_until(&mut s, "refreshed listing", |s| {
        s.dirs.entries(&uri("/d")).is_some_and(|e| e.len() == 2)
    });
}
