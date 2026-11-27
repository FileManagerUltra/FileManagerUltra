//! Session-level editor tests (capability `text-editor`): load, save, conflicts,
//! autosave, the leave warning and the close guard, on local-, SMB- and SFTP-shaped
//! in-memory backends.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use fm_layout::{PaneId, PaneKind};
use fm_testkit::{FaultFs, MemFs};
use fm_vfs::{Entry, EntryKind, FmError, Scheme, Vfs, VfsUri};
use serde_json::json;

use crate::editor_doc::Phase;
use crate::session::Session;
use crate::workspace::AfterPrompt;

fn uri(s: &str) -> VfsUri {
    VfsUri::parse(s).expect("uri")
}

fn settle(s: &mut Session, what: &str, until: impl Fn(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !until(s) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        s.poll();
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// A session with a Files pane (1) and a Preview pane beside it.
fn session(fs: Arc<dyn Vfs>, root: &VfsUri) -> (Session, PaneId, PaneId) {
    let mut s = Session::new().expect("session");
    if root.scheme() == Scheme::File {
        s.backends.register(fs);
    } else {
        s.backends.register_at(root.authority(), fs);
    }
    let files = s.layout.focused();
    s.dispatch("view.preview.toggle", &json!(null))
        .expect("preview pane");
    let preview = s
        .layout
        .panes()
        .into_iter()
        .find(|p| s.pane_kind(*p) == Some(PaneKind::Preview))
        .expect("preview");
    (s, files, preview)
}

fn open(s: &mut Session, file: &VfsUri) -> PaneId {
    s.dispatch("editor.open", &json!({ "uri": file.as_str() }))
        .expect("open");
    let pane = s.layout.focused();
    settle(s, "load", |s| {
        s.editor(pane).is_some_and(|d| d.phase == Phase::Ready)
    });
    pane
}

fn type_text(s: &mut Session, pane: PaneId, text: &str) {
    s.dispatch(
        "editor.input",
        &json!({"op": "insert", "text": text, "pane": pane}),
    )
    .expect("type");
}

fn saved(s: &Session, pane: PaneId) -> bool {
    s.editor(pane)
        .is_some_and(|d| !d.dirty() && d.phase == Phase::Ready)
}

#[test]
fn edits_round_trip_on_local_smb_and_sftp_backends() {
    for root in ["/w", "smb://nas/share/w", "sftp://host/home/u/w"] {
        let root = uri(root);
        let mem = MemFs::new().with_scheme(root.scheme());
        let file = root.join("notes.txt").expect("file");
        mem.put_file(&file, b"one\r\ntwo\r\n").expect("put");
        let mem = Arc::new(mem);
        let (mut s, _, preview) = session(mem.clone(), &root);
        let pane = open(&mut s, &file);
        assert_eq!(pane, preview, "the Preview pane edits the file");
        let doc = s.editor(pane).expect("doc");
        assert_eq!(doc.buffer.to_text(), "one\r\ntwo\r\n");
        assert_eq!(doc.eol, fm_editor::Eol::CrLf);
        s.dispatch("editor.input", &json!({"op": "move", "to": "doc_end"}))
            .expect("end");
        type_text(&mut s, pane, "three\n");
        assert!(s.editor(pane).expect("doc").dirty());
        s.dispatch("editor.save", &json!(null)).expect("save");
        settle(&mut s, "save", |s| saved(s, pane));
        assert_eq!(
            mem.get_file(&file).expect("get"),
            b"one\r\ntwo\r\nthree\r\n",
            "{root}: line endings kept"
        );
        assert_eq!(
            mem.read_dir(&root).expect("list").len(),
            1,
            "no temp file left"
        );
    }
}

#[test]
fn a_changed_file_is_never_overwritten_silently() {
    let mem = Arc::new(MemFs::new());
    let file = uri("/w/a.txt");
    mem.put_file(&file, b"mine").expect("put");
    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "!");
    mem.put_file(&file, b"theirs, longer")
        .expect("outside change");
    s.dispatch("editor.save", &json!(null)).expect("save");
    settle(&mut s, "conflict", |s| {
        s.editor(pane).is_some_and(|d| d.conflict.is_some())
    });
    assert_eq!(mem.get_file(&file).expect("get"), b"theirs, longer");
    s.dispatch("editor.resolve", &json!({"choice": "overwrite"}))
        .expect("overwrite");
    settle(&mut s, "overwrite", |s| saved(s, pane));
    assert_eq!(mem.get_file(&file).expect("get"), b"!mine");
}

#[test]
fn focus_loss_autosaves_and_warns_when_autosave_is_off() {
    let mem = Arc::new(MemFs::new());
    let file = uri("/w/a.rs");
    mem.put_file(&file, b"fn main() {}\n").expect("put");
    let (mut s, files, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "// ");
    s.layout.focus_pane(files).expect("focus files");
    settle(&mut s, "autosave", |s| saved(s, pane));
    assert_eq!(mem.get_file(&file).expect("get"), b"// fn main() {}\n");

    s.dispatch("prefs.editor_autosave", &json!({"enabled": false}))
        .expect("off");
    s.layout.focus_pane(pane).expect("back");
    s.poll();
    type_text(&mut s, pane, "x");
    s.layout.focus_pane(files).expect("leave");
    s.poll();
    assert!(s.editor(pane).expect("doc").leave_prompt, "warning shown");
    assert!(s.command_enabled("editor.resolve"));
    s.dispatch("editor.resolve", &json!({"choice": "cancel"}))
        .expect("cancel");
    assert_eq!(s.layout.focused(), pane, "Cancel goes back to the editor");
    assert!(s.editor(pane).expect("doc").dirty(), "nothing lost");
    s.layout.focus_pane(files).expect("leave again");
    s.poll();
    s.dispatch("editor.resolve", &json!({"choice": "save"}))
        .expect("save");
    settle(&mut s, "save", |s| saved(s, pane));
    assert_eq!(mem.get_file(&file).expect("get"), b"// xfn main() {}\n");
}

#[test]
fn window_blur_saves_every_dirty_editor() {
    let mem = Arc::new(MemFs::new());
    let file = uri("/w/a.txt");
    mem.put_file(&file, b"a").expect("put");
    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "b");
    s.dispatch("editor.window_blur", &json!(null))
        .expect("blur");
    settle(&mut s, "blur save", |s| saved(s, pane));
    assert_eq!(mem.get_file(&file).expect("get"), b"ba");
}

#[test]
fn a_failed_save_keeps_the_text_dirty_and_does_not_retry() {
    let fault = FaultFs::new(MemFs::new());
    let file = uri("/w/a.txt");
    fault.inner().put_file(&file, b"original").expect("put");
    let (mut s, files, _) = session(Arc::new(fault.clone()), &uri("/w"));
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "new ");
    fault.faults().fail_write_after = Some(1);
    s.layout.focus_pane(files).expect("leave");
    settle(&mut s, "failed save", |s| {
        s.editor(pane).is_some_and(|d| d.save_error.is_some())
    });
    let doc = s.editor(pane).expect("doc");
    assert!(doc.dirty(), "edits kept");
    assert!(
        s.notice
            .as_deref()
            .is_some_and(|n| n.contains("was not saved"))
    );
    assert_eq!(fault.inner().get_file(&file).expect("get"), b"original");
    assert_eq!(fault.inner().read_dir(&uri("/w")).expect("list").len(), 1);
    // Focus going back and forth does not hammer the same failing save.
    s.notice = None;
    s.layout.focus_pane(pane).expect("back");
    s.poll();
    s.layout.focus_pane(files).expect("leave");
    s.poll();
    assert!(s.editor(pane).expect("doc").phase == Phase::Ready);
    assert!(s.notice.is_none(), "no second attempt");
}

#[test]
fn quitting_saves_with_autosave_and_asks_without() {
    let mem = Arc::new(MemFs::new());
    let file = uri("/w/a.txt");
    mem.put_file(&file, b"a").expect("put");
    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "1");
    s.close_or_ask(AfterPrompt::Quit).expect("quit");
    settle(&mut s, "quit after save", |s| !s.running);
    assert_eq!(mem.get_file(&file).expect("get"), b"1a");

    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    s.prefs.editor.autosave = false;
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "2");
    s.close_or_ask(AfterPrompt::Quit).expect("quit");
    let prompt = s.unsaved_prompt.clone().expect("asked");
    assert_eq!(prompt.editors, vec![(pane, "a.txt".to_string())]);
    assert!(s.running);
    s.workspace_resolve_prompt("save", false).expect("save");
    settle(&mut s, "quit after prompt save", |s| !s.running);
    assert_eq!(mem.get_file(&file).expect("get"), b"21a");
}

#[test]
fn unsaved_edits_pin_the_pane_and_clean_ones_follow() {
    let mem = Arc::new(MemFs::new());
    let a = uri("/w/a.txt");
    let b = uri("/w/b.txt");
    mem.put_file(&a, b"a").expect("put");
    mem.put_file(&b, b"b").expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    s.prefs.editor.autosave = false;
    let pane = open(&mut s, &a);
    type_text(&mut s, pane, "x");
    // The source pointed at the edited file (normally armed by the first poll).
    if let Some(doc) = s.editor_mut(pane) {
        doc.followed = Some(a.clone());
    }
    let other = Entry::new(b, EntryKind::File);
    assert!(s.editor_holds(pane, std::slice::from_ref(&other)));
    assert!(s.editor(pane).expect("doc").leave_prompt, "warning instead");
    assert_eq!(s.editor(pane).expect("doc").uri, a, "same file");
    s.editor_resolve(pane, "cancel").expect("cancel");
    assert!(s.editor_holds(pane, std::slice::from_ref(&other)));
    assert!(
        !s.editor(pane).expect("doc").leave_prompt,
        "not asked again for the same file"
    );
    s.editor_resolve(pane, "reload").expect("discard by reload");
    settle(&mut s, "reload", |s| saved(s, pane));
    assert!(!s.editor_holds(pane, std::slice::from_ref(&other)));
}

#[test]
fn a_clean_buffer_reloads_when_the_file_changes() {
    let mem = Arc::new(MemFs::new());
    let file = uri("/w/a.txt");
    mem.put_file(&file, b"one\ntwo\n").expect("put");
    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &file);
    s.dispatch("editor.goto_line", &json!({"line": 2}))
        .expect("goto");
    mem.put_file(&file, b"one\ntwo\nthree\n").expect("outside");
    s.dir_changed(&uri("/w"));
    settle(&mut s, "reload", |s| {
        s.editor(pane)
            .is_some_and(|d| d.phase == Phase::Ready && d.buffer.to_text().ends_with("three\n"))
    });
    let doc = s.editor(pane).expect("doc");
    assert_eq!(doc.buffer.char_to_pos(doc.caret).expect("pos").line, 1);

    type_text(&mut s, pane, "dirty ");
    mem.put_file(&file, b"again changed\n").expect("outside");
    s.dir_changed(&uri("/w"));
    settle(&mut s, "bar", |s| {
        s.editor(pane).is_some_and(|d| d.changed_on_disk)
    });
    assert!(s.editor(pane).expect("doc").dirty(), "dirty text kept");
}

#[test]
fn binary_files_and_archives_are_not_edited() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/blob.dat"), b"ab\0cd").expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    let pane = open(&mut s, &uri("/w/blob.dat"));
    let doc = s.editor(pane).expect("doc");
    assert_eq!(doc.blocked, Some(crate::editor_io::Blocked::Binary));
    assert!(
        s.dispatch("editor.input", &json!({"op": "insert", "text": "x"}))
            .is_err()
    );
    assert!(matches!(
        s.dispatch("editor.open", &json!({"uri": "/w/photo.png"})),
        Err(FmError::InvalidParams(_))
    ));
}

#[test]
fn commands_refuse_bad_params_and_the_wrong_pane() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.txt"), b"a\nb\n").expect("put");
    let (mut s, files, _) = session(mem, &uri("/w"));
    s.layout.focus_pane(files).expect("files");
    assert!(!s.command_enabled("editor.save"));
    assert!(s.dispatch("editor.save", &json!(null)).is_err());
    let pane = open(&mut s, &uri("/w/a.txt"));
    for (id, params) in [
        ("editor.goto_line", json!("12")),
        ("editor.goto_line", json!({})),
        ("editor.goto_line", json!({"line": 0})),
        ("editor.goto_line", json!({"text": "abc"})),
        ("editor.input", json!({"op": "fly"})),
        ("editor.input", json!({"op": "insert"})),
        ("editor.input", json!({"op": "move", "to": "sideways"})),
        ("editor.set_eol", json!({"eol": "cr"})),
        ("editor.set_encoding", json!({"encoding": "ebcdic"})),
        ("editor.resolve", json!({"choice": "maybe"})),
        ("editor.open", json!({"uri": 7})),
        ("editor.open", json!({"uri": "/w/a.txt", "activate": "yes"})),
        ("editor.find", json!({"query": 1})),
        ("editor.save", json!({"bogus": true})),
    ] {
        assert!(
            matches!(s.dispatch(id, &params), Err(FmError::InvalidParams(_))),
            "{id} {params}"
        );
    }
    assert_eq!(s.editor(pane).expect("doc").caret, 0, "caret unmoved");
    s.dispatch("editor.goto_line", &json!({"line": 99_999_999}))
        .expect("past the end");
    let doc = s.editor(pane).expect("doc");
    assert_eq!(doc.buffer.char_to_pos(doc.caret).expect("pos").line, 2);
}

#[test]
fn find_and_replace_all_is_one_undo_step() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.txt"), "foo bar foo\nfoo\n".as_bytes())
        .expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    let pane = open(&mut s, &uri("/w/a.txt"));
    s.dispatch(
        "editor.find",
        &json!({"query": "foo", "replacement": "baz"}),
    )
    .expect("find");
    assert_eq!(s.editor(pane).expect("doc").find.matches.len(), 3);
    s.dispatch("editor.replace", &json!({"all": true}))
        .expect("replace all");
    assert_eq!(
        s.editor(pane).expect("doc").buffer.to_text(),
        "baz bar baz\nbaz\n"
    );
    s.dispatch("editor.undo", &json!(null)).expect("undo");
    assert_eq!(
        s.editor(pane).expect("doc").buffer.to_text(),
        "foo bar foo\nfoo\n"
    );
    s.dispatch("editor.find", &json!({"query": "(", "regex": true}))
        .expect("bad pattern shows an error, no search");
    assert!(s.editor(pane).expect("doc").find.error.is_some());
}

#[test]
fn undo_depth_applies_to_open_editors() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.txt"), b"").expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    let pane = open(&mut s, &uri("/w/a.txt"));
    for _ in 0..80 {
        s.dispatch("editor.input", &json!({"op": "newline"}))
            .expect("edit");
    }
    s.dispatch("prefs.editor_undo_steps", &json!({"steps": 50}))
        .expect("depth");
    let doc = s.editor(pane).expect("doc");
    assert_eq!(doc.undo.undo_steps(), 50);
    assert!(doc.undo.truncated());
}

#[test]
fn toggling_back_to_preview_saves_first() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.md"), b"# t\n").expect("put");
    let (mut s, _, _) = session(mem.clone(), &uri("/w"));
    let pane = open(&mut s, &uri("/w/a.md"));
    type_text(&mut s, pane, "x");
    s.dispatch("editor.toggle", &json!(null)).expect("toggle");
    settle(&mut s, "leave", |s| s.editor(pane).is_none());
    assert_eq!(mem.get_file(&uri("/w/a.md")).expect("get"), b"x# t\n");
    assert!(s.layout.state(pane).expect("pane").edit.is_none());
}

#[test]
fn pane_state_restores_edit_mode() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.txt"), b"1\n2\n3\n").expect("put");
    let (mut s, _, preview) = session(mem, &uri("/w"));
    s.layout.state_mut(preview).expect("pane").edit = Some(fm_layout::EditViewState {
        uri: uri("/w/a.txt"),
        line: 2,
        col: 1,
        top: 0,
        wrap: true,
        folds: Vec::new(),
    });
    settle(&mut s, "restore", |s| {
        s.editor(preview).is_some_and(|d| d.phase == Phase::Ready)
    });
    let doc = s.editor(preview).expect("doc");
    let pos = doc.buffer.char_to_pos(doc.caret).expect("pos");
    assert_eq!((pos.line, pos.col), (2, 1));
    assert!(doc.wrap);
}

#[test]
fn f4_honours_the_open_preference() {
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/a.log"), b"x").expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    assert_eq!(
        s.editor_target_for(&uri("/w/a.log")),
        Some(crate::editor_prefs::EditorTarget::Internal)
    );
    s.dispatch(
        "prefs.editor_open",
        &json!({"kind": "log", "target": "external"}),
    )
    .expect("pref");
    assert_eq!(
        s.editor_target_for(&uri("/w/a.log")),
        Some(crate::editor_prefs::EditorTarget::External)
    );
}

/// A Files listing that arrives after the editor opened is not a move of the source:
/// it must not autosave or warn. A real move afterwards still does.
#[test]
fn a_late_source_listing_does_not_count_as_leaving() {
    let root = uri("/w");
    let mem = MemFs::new();
    let file = root.join("notes.txt").expect("file");
    let other = root.join("other.txt").expect("other");
    mem.put_file(&file, b"one\n").expect("put");
    mem.put_file(&other, b"two\n").expect("put");
    let (mut s, _files, _preview) = session(Arc::new(mem), &root);
    let pane = open(&mut s, &file);
    type_text(&mut s, pane, "draft ");
    if let Some(doc) = s.editor_mut(pane) {
        doc.followed = None;
    }
    let folder = Entry::new(root.clone(), EntryKind::Dir);
    assert!(s.editor_holds(pane, std::slice::from_ref(&folder)));
    let doc = s.editor(pane).expect("doc");
    assert!(
        doc.dirty() && doc.phase == Phase::Ready,
        "no autosave on arming"
    );
    assert_eq!(doc.followed.as_ref(), Some(&root));
    assert!(
        s.editor_holds(pane, std::slice::from_ref(&folder)),
        "unchanged: no move"
    );
    assert!(s.editor(pane).expect("doc").phase == Phase::Ready);
    let next = Entry::new(other, EntryKind::File);
    assert!(s.editor_holds(pane, &[next]));
    settle(&mut s, "autosave on a real move", |s| {
        saved(s, pane) || s.editor(pane).is_none_or(|d| d.uri != file)
    });
}

#[test]
fn files_of_unknown_type_open_as_plain_text_and_only_double_clicked_binaries_leave() {
    use crate::editor_io::Blocked;
    let mem = Arc::new(MemFs::new());
    mem.put_file(&uri("/w/notes.xyz"), b"plain words\n")
        .expect("put");
    mem.put_file(&uri("/w/panel.md.bak-20261123b"), b"# Title\n")
        .expect("put");
    let (mut s, _, _) = session(mem, &uri("/w"));
    let pane = open(&mut s, &uri("/w/notes.xyz"));
    let doc = s.editor(pane).expect("doc");
    assert_eq!(
        doc.blocked, None,
        "unknown extension, text inside: editable"
    );
    assert_eq!(doc.lang, None, "plain-text mode");
    assert!(
        !doc.activated,
        "F4 and the command default to not activated"
    );
    let pane = open(&mut s, &uri("/w/panel.md.bak-20261123b"));
    assert_eq!(
        s.editor(pane).and_then(|d| d.lang),
        Some(fm_editor::Lang::Markdown),
        "a backup is coloured like its original"
    );
    // Only a double-clicked binary is handed to its default application.
    assert!(crate::editor_hub::hands_off_to_app(true, &Blocked::Binary));
    assert!(!crate::editor_hub::hands_off_to_app(
        false,
        &Blocked::Binary
    ));
    assert!(!crate::editor_hub::hands_off_to_app(
        true,
        &Blocked::TooLarge(1)
    ));
    assert!(!crate::editor_hub::hands_off_to_app(
        true,
        &Blocked::NotAFile
    ));
}
