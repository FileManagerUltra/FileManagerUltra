//! Pane kinds and terminal panes through `Session` commands.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::Session;
use fm_layout::PaneKind;
use fm_vfs::VfsUri;

fn session_in(dir: &std::path::Path) -> Session {
    let mut s = Session::new().expect("session");
    let uri = VfsUri::parse(&dir.to_string_lossy()).expect("uri");
    let pane = s.layout.focused();
    s.layout.state_mut(pane).expect("pane").navigate(uri);
    s
}

fn kind_of(s: &Session) -> PaneKind {
    s.focused_kind().expect("kind")
}

fn screen_text(s: &Session, pane: u64) -> String {
    let snap = s.terminal_snapshot(pane).expect("terminal");
    snap.text_between((0, 0), (snap.rows - 1, snap.cols - 1))
}

fn wait_for(s: &mut Session, pane: u64, needle: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        s.poll();
        if screen_text(s, pane).contains(needle) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn has_pty(s: &mut Session) -> bool {
    s.dispatch("pane.kind.set", &json!({"kind": "terminal"}))
        .is_ok()
}

#[test]
fn new_panes_are_files_and_unknown_kinds_are_rejected() {
    let mut s = Session::new().expect("session");
    assert_eq!(kind_of(&s), PaneKind::Files);
    for bad in [
        json!(null),
        json!("terminal"),
        json!({}),
        json!({"kind": 3}),
        json!({"kind": "floppy"}),
        json!({"kind": "terminal", "pane": "x"}),
        json!({"kind": "files", "pane": 9999}),
    ] {
        assert!(s.dispatch("pane.kind.set", &bad).is_err(), "{bad}");
    }
    assert_eq!(kind_of(&s), PaneKind::Files);
}

#[test]
fn network_kind_roots_the_pane_at_the_network_folder() {
    let mut s = Session::new().expect("session");
    s.backends
        .register(std::sync::Arc::new(fm_discovery::NetworkFs::fixed(
            fm_discovery::HostList::new(),
        )));
    s.dispatch("pane.kind.set", &json!({ "kind": "network" }))
        .expect("network");
    assert_eq!(kind_of(&s), PaneKind::Network);
    let pane = s.layout.focused();
    let st = s.layout.state(pane).expect("pane");
    assert_eq!(st.location, fm_vfs::VfsUri::network());
}

#[test]
fn files_state_survives_a_round_trip_through_a_terminal() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
    let mut s = session_in(dir.path());
    let pane = s.layout.focused();
    let before = s.layout.state(pane).expect("pane").clone();
    if !has_pty(&mut s) {
        return;
    }
    assert_eq!(kind_of(&s), PaneKind::Terminal);
    s.dispatch("pane.kind.set", &json!({"kind": "files"}))
        .expect("back");
    let after = s.layout.state(pane).expect("pane");
    assert_eq!(after.kind, PaneKind::Files);
    assert_eq!(after.location, before.location);
    assert_eq!(after.history, before.history);
    assert_eq!(after.listing, before.listing);
    assert!(
        s.terminal(pane).is_none(),
        "leaving a terminal ends its shell"
    );
}

#[cfg(unix)]
#[test]
fn terminal_starts_in_the_pane_directory_and_echoes_input() {
    let dir = tempfile::tempdir().expect("tmp");
    let canonical = std::fs::canonicalize(dir.path()).expect("canon");
    let mut s = session_in(&canonical);
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("term.input", &json!({"data": "pwd; echo fm-$((6*7))\r"}))
        .expect("input");
    assert!(wait_for(&mut s, pane, "fm-42"));
    let leaf = canonical
        .file_name()
        .expect("leaf")
        .to_string_lossy()
        .into_owned();
    assert!(screen_text(&s, pane).contains(&leaf));
}

#[test]
fn file_commands_are_disabled_on_a_terminal_pane() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(dir.path().join("a"), b"a").expect("file");
    let mut s = session_in(dir.path());
    if !has_pty(&mut s) {
        return;
    }
    for id in [
        "file.copy",
        "file.move",
        "file.delete",
        "file.mkdir",
        "file.newfile",
        "nav.goto",
    ] {
        let spec = s.registry.get(id).expect(id);
        assert!(!(spec.enabled)(&s), "{id} must be disabled");
    }
    let spec = s.registry.get("term.input").expect("term.input");
    assert!((spec.enabled)(&s));
}

#[test]
fn a_terminal_pane_is_never_the_other_pane_destination() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = session_in(dir.path());
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let panes = s.layout.panes();
    let (a, b) = (panes[0], panes[1]);
    s.layout.focus_pane(b).expect("focus");
    if s.dispatch("pane.kind.set", &json!({"kind": "terminal", "pane": b}))
        .is_err()
    {
        return;
    }
    s.layout.focus_pane(a).expect("focus");
    assert!(fm_engine::file_cmds::other_pane_dir(&s).is_none());
}

#[cfg(unix)]
#[test]
fn cd_here_quotes_hostile_directory_names() {
    let dir = tempfile::tempdir().expect("tmp");
    let hostile = dir.path().join("it's; echo pwned #");
    std::fs::create_dir(&hostile).expect("mkdir");
    let mut s = session_in(dir.path());
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let panes = s.layout.panes();
    let (files, term) = (panes[0], panes[1]);
    let hostile_uri = VfsUri::parse(&hostile.to_string_lossy()).expect("uri");
    s.layout
        .state_mut(files)
        .expect("pane")
        .navigate(hostile_uri);
    s.layout.focus_pane(term).expect("focus");
    if s.dispatch("pane.kind.set", &json!({"kind": "terminal", "pane": term}))
        .is_err()
    {
        return;
    }
    s.layout.focus_pane(term).expect("focus");
    s.dispatch("term.cd_here", &json!(null)).expect("cd_here");
    s.dispatch("term.input", &json!({"data": "echo [$PWD]\r"}))
        .expect("input");
    assert!(
        wait_for(&mut s, term, "[") && {
            let leaf = hostile
                .file_name()
                .expect("leaf")
                .to_string_lossy()
                .into_owned();
            wait_for(&mut s, term, &format!("{leaf}]"))
        }
    );
    assert!(
        !screen_text(&s, term).contains("pwned\n"),
        "no injected command ran"
    );
}

#[test]
fn terminal_commands_reject_bad_params_and_wrong_panes() {
    let mut s = Session::new().expect("session");
    for id in ["term.input", "term.paste"] {
        assert!(
            s.dispatch(id, &json!({"data": "x", "text": "x"})).is_err(),
            "{id} on a Files pane"
        );
    }
    assert!(s.dispatch("term.restart", &json!({"pane": 1})).is_err());
    if !has_pty(&mut s) {
        return;
    }
    assert!(s.dispatch("term.input", &json!({"data": 5})).is_err());
    assert!(s.dispatch("term.input", &json!("x")).is_err());
    assert!(s.dispatch("term.input", &json!({})).is_err());
    let big = "x".repeat(fm_engine::term_cmds::MAX_KEYS_BYTES + 1);
    assert!(s.dispatch("term.input", &json!({"data": big})).is_err());
    let huge = "x".repeat(fm_engine::term_cmds::MAX_PASTE_BYTES + 1);
    assert!(s.dispatch("term.paste", &json!({"text": huge})).is_err());
    assert!(s.dispatch("term.paste", &json!({"text": "ok"})).is_ok());
}

#[cfg(unix)]
#[test]
fn closing_a_terminal_pane_kills_its_shell() {
    let mut s = Session::new().expect("session");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    let term = s.layout.focused();
    if !has_pty(&mut s) {
        return;
    }
    assert!(s.terminal(term).is_some());
    s.dispatch("pane.close", &json!(null)).expect("close");
    assert!(!s.has_terminals());
}

#[cfg(unix)]
#[test]
fn restart_replaces_an_exited_shell() {
    let mut s = Session::new().expect("session");
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("term.input", &json!({"data": "exit 3\r"}))
        .expect("input");
    let deadline = Instant::now() + Duration::from_secs(10);
    while fm_engine::term_cmds::exit_code(&s, pane).is_none() && Instant::now() < deadline {
        s.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fm_engine::term_cmds::exit_code(&s, pane), Some(3));
    s.dispatch("term.restart", &json!(null)).expect("restart");
    assert_eq!(fm_engine::term_cmds::exit_code(&s, pane), None);
}

#[cfg(unix)]
#[test]
fn exiting_the_shell_closes_the_pane_unless_it_is_the_last() {
    let mut s = Session::new().expect("session");
    s.dispatch("pane.split.horizontal", &json!(null))
        .expect("split");
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("term.input", &json!({"data": "exit\r"}))
        .expect("input");
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.layout.panes().contains(&pane) && Instant::now() < deadline {
        s.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!s.layout.panes().contains(&pane), "pane closed");
    assert!(!s.has_terminals());
}

#[test]
fn copy_hands_the_selection_to_the_clipboard_slot() {
    use fm_engine::term_cmds::TermSelection;
    let mut s = Session::new().expect("session");
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("term.input", &json!({"data": "echo copy-me\r"}))
        .expect("input");
    assert!(wait_for(&mut s, pane, "copy-me\n") || wait_for(&mut s, pane, "copy-me"));
    s.dispatch("term.copy", &json!(null)).expect("noop");
    assert!(s.clipboard_out.is_none(), "nothing selected");
    s.terminal_select(Some(TermSelection {
        pane,
        anchor: (0, 0),
        head: (23, 79),
    }));
    s.dispatch("term.copy", &json!(null)).expect("copy");
    assert!(
        s.clipboard_out
            .as_deref()
            .is_some_and(|t| t.contains("copy-me"))
    );
}

#[test]
fn leaving_an_idle_terminal_ends_it_without_asking() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = session_in(dir.path());
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("pane.kind.set", &json!({"kind": "files"}))
        .expect("switch");
    assert_eq!(kind_of(&s), PaneKind::Files);
    assert!(s.terminal(pane).is_none());
    assert!(s.pending_kind.is_none());
}

#[cfg(unix)]
#[test]
fn leaving_a_busy_terminal_asks_first_and_confirm_ends_it() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = session_in(dir.path());
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    s.dispatch("term.input", &json!({"data": "sleep 30\r"}))
        .expect("input");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !s.terminal_busy(pane) && Instant::now() < deadline {
        s.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.terminal_busy(pane), "sleep should own the terminal");

    s.dispatch("pane.kind.set", &json!({"kind": "files"}))
        .expect("switch");
    assert_eq!(kind_of(&s), PaneKind::Terminal);
    assert_eq!(s.pending_kind, Some((pane, PaneKind::Files)));
    assert!(s.terminal(pane).is_some());

    s.dispatch("pane.kind.set", &json!({"kind": "files", "confirm": true}))
        .expect("confirmed");
    assert_eq!(kind_of(&s), PaneKind::Files);
    assert!(s.terminal(pane).is_none());
    assert!(s.pending_kind.is_none());
}

#[cfg(unix)]
#[test]
fn dropped_paths_are_pasted_quoted_without_running_anything() {
    let dir = tempfile::tempdir().expect("tmp");
    let canonical = std::fs::canonicalize(dir.path()).expect("canon");
    let mut s = session_in(&canonical);
    if !has_pty(&mut s) {
        return;
    }
    let pane = s.layout.focused();
    let hostile = canonical.join("a b; touch pwned's");
    std::fs::write(&hostile, b"x").expect("file");
    s.dispatch(
        "term.drop_paths",
        &json!({"paths": [hostile.to_string_lossy(), "smb://host/share/x"], "pane": pane}),
    )
    .expect("drop");
    assert!(s.notice.is_some(), "the remote item is reported as skipped");
    assert!(wait_for(&mut s, pane, "pwned"));
    std::thread::sleep(Duration::from_millis(200));
    s.poll();
    assert!(!canonical.join("pwned").exists(), "nothing ran");
    let text = screen_text(&s, pane);
    assert!(text.contains("touch pwned"), "{text}");
}

#[test]
fn drop_paths_rejects_bad_params_and_remote_only_drops() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = session_in(dir.path());
    if !has_pty(&mut s) {
        return;
    }
    for p in [
        json!(null),
        json!({}),
        json!({"paths": "x"}),
        json!({"paths": []}),
        json!({"paths": ["smb://h/s/x"]}),
        json!({"paths": vec!["/x"; 501]}),
    ] {
        assert!(s.dispatch("term.drop_paths", &p).is_err(), "{p}");
    }
}
