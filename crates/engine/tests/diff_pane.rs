#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Diff panes: loading, navigation commands, hostile params.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use fm_engine::diff_cmds::{DiffStatus, Outcome};
use fm_engine::{Params, Session};
use fm_layout::PaneKind;
use serde_json::json;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn unreachable_ready() -> ! {
    std::process::exit(101)
}

fn ready(s: &mut Session) -> std::sync::Arc<Outcome> {
    let pane = s.layout.focused();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        s.poll();
        if let Some(DiffStatus::Ready(o)) = s.diff_view(pane).map(|v| v.status.clone()) {
            return o;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    unreachable_ready()
}

#[test]
fn compares_working_file_with_head_and_navigates() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), "1\n2\n3\n4\n5\n6\n7\n8\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "i"]);
    std::fs::write(d.join("a.txt"), "1\nX\n3\n4\n5\n6\n7\nY\n").unwrap();
    let mut s = Session::new().unwrap();
    let path = d.join("a.txt").to_string_lossy().into_owned();
    s.dispatch("diff.compare.head", &json!({ "path": path }))
        .unwrap();
    assert_eq!(s.focused_kind(), Some(PaneKind::Diff));
    let Outcome::Text(model) = ready(&mut s).as_ref().clone() else {
        unreachable_ready()
    };
    assert_eq!(model.changes.len(), 2);

    s.dispatch("diff.next", &Params::Null).unwrap();
    s.dispatch("diff.next", &Params::Null).unwrap();
    let pane = s.layout.focused();
    assert_eq!(
        s.layout.state(pane).unwrap().diff.as_ref().unwrap().current,
        1
    );
    s.dispatch("diff.prev", &Params::Null).unwrap();
    assert_eq!(
        s.layout.state(pane).unwrap().diff.as_ref().unwrap().current,
        0
    );

    s.dispatch("diff.swap", &Params::Null).unwrap();
    assert!(matches!(ready(&mut s).as_ref(), Outcome::Text(_)));
    s.dispatch("diff.mode", &json!({ "mode": "unified" }))
        .unwrap();
    s.dispatch("diff.eol.toggle", &Params::Null).unwrap();
}

#[test]
fn missing_and_oversized_sides() {
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    let big = tmp.path().join("big");
    std::fs::write(&a, "x\n").unwrap();
    std::fs::write(&big, vec![b'a'; 3 << 20]).unwrap();
    let mut s = Session::new().unwrap();
    s.dispatch(
        "diff.open",
        &json!({"left": a.to_string_lossy(), "right": tmp.path().join("none").to_string_lossy()}),
    )
    .unwrap();
    assert!(matches!(ready(&mut s).as_ref(), Outcome::Text(_)));
    s.dispatch(
        "diff.open",
        &json!({"left": a.to_string_lossy(), "right": big.to_string_lossy()}),
    )
    .unwrap();
    let pane = s.layout.focused();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        if let Some(DiffStatus::Failed(_)) = s.diff_view(pane).map(|v| v.status.clone()) {
            break;
        }
        assert!(Instant::now() < deadline, "size cap not enforced");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn hostile_params_are_rejected() {
    let mut s = Session::new().unwrap();
    for (id, p) in [
        ("diff.open", json!(null)),
        ("diff.open", json!({"left": "relative", "right": "/x"})),
        ("diff.open", json!({"left": 5, "right": "/x"})),
        ("diff.open", json!({"path": "/tmp", "rev": "--output=x"})),
        (
            "diff.open",
            json!({"path": "/nonexistent/x/y", "rev": "HEAD"}),
        ),
        ("diff.compare.head", json!({"path": 3})),
        ("diff.compare.marked", json!({})),
        ("diff.mode", json!({"mode": "bogus"})),
        ("diff.rev", json!({"side": "up", "rev": "HEAD"})),
        ("diff.side", json!({"side": "left", "path": "rel"})),
        ("diff.swap", json!({})),
        ("diff.next", json!({})),
    ] {
        assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
    }
    assert!(
        s.dispatch("pane.kind.set", &json!({"kind": "diff"}))
            .is_err()
    );
}
