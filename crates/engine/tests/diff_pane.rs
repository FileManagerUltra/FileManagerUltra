#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The Preview pane's diff: default view, navigation commands, hostile params.

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

fn exit_timeout(what: &str) -> ! {
    eprintln!("timed out: {what}");
    std::process::exit(101)
}

fn pump(s: &mut Session, what: &str, mut done: impl FnMut(&mut Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        s.poll();
        if done(s) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    exit_timeout(what)
}

fn open_in_preview(s: &mut Session, path: &Path) -> u64 {
    let dir = fm_vfs::VfsUri::parse(&path.parent().unwrap().to_string_lossy()).unwrap();
    let files = s.layout.focused();
    s.layout.state_mut(files).unwrap().navigate(dir.clone());
    s.ensure_read(dir).unwrap();
    pump(s, "listing", |s| s.pane_view_ready(files));
    let uri = fm_vfs::VfsUri::parse(&path.to_string_lossy()).unwrap();
    s.dispatch("preview.open", &json!({ "uri": uri.to_string() }))
        .unwrap();
    s.layout
        .panes()
        .into_iter()
        .find(|p| s.pane_kind(*p) == Some(PaneKind::Preview))
        .unwrap()
}

fn repo(committed: &str, working: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), committed).unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "i"]);
    std::fs::write(d.join("a.txt"), working).unwrap();
    tmp
}

#[test]
fn a_modified_file_shows_its_diff_by_default_and_navigates() {
    let tmp = repo("1\n2\n3\n4\n5\n6\n7\n8\n", "1\nX\n3\n4\n5\n6\n7\nY\n");
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    pump(&mut s, "diff by default", |s| {
        s.pane_diff(pane).is_some_and(|d| d.active)
    });
    let mut model = None;
    pump(&mut s, "diff ready", |s| {
        if let Some(DiffStatus::Ready(o)) = s.diff_view(pane).map(|v| v.status.clone())
            && let Outcome::Text(m) = o.as_ref()
        {
            model = Some(m.clone());
            return true;
        }
        false
    });
    assert_eq!(model.unwrap().changes.len(), 2);

    let current = |s: &Session| s.pane_diff(pane).unwrap().state.current;
    s.dispatch("preview.diff.next", &json!({})).unwrap();
    s.dispatch("preview.diff.next", &json!({})).unwrap();
    assert_eq!(current(&s), 1);
    s.dispatch("preview.diff.prev", &json!({})).unwrap();
    assert_eq!(current(&s), 0);
    s.dispatch("preview.diff.mode", &json!({"mode": "unified"}))
        .unwrap();
    s.dispatch("preview.diff.eol", &Params::Null).unwrap();

    // The user's choice sticks until the selection changes.
    s.dispatch("preview.diff.toggle", &json!({})).unwrap();
    s.poll();
    assert!(!s.pane_diff(pane).unwrap().active);
    s.dispatch("preview.diff.toggle", &json!({})).unwrap();
    assert!(s.pane_diff(pane).unwrap().active);
}

#[test]
fn an_unmodified_file_shows_the_preview_until_diff_is_chosen() {
    let tmp = repo("same\n", "same\n");
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    pump(&mut s, "diff available", |s| s.pane_diff(pane).is_some());
    for _ in 0..10 {
        s.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!s.pane_diff(pane).unwrap().active);
    s.dispatch("preview.diff.toggle", &json!({"show": true}))
        .unwrap();
    pump(&mut s, "no changes", |s| {
        matches!(
            s.diff_view(pane).map(|v| v.status.clone()),
            Some(DiffStatus::Ready(o)) if matches!(o.as_ref(), Outcome::Text(m) if m.identical())
        )
    });
}

#[test]
fn a_file_outside_a_repository_has_no_diff() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.txt"), "x\n").unwrap();
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    for _ in 0..10 {
        s.poll();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(s.pane_diff(pane).is_none());
    assert!(s.dispatch("preview.diff.toggle", &json!({})).is_err());
}

#[test]
fn hostile_params_are_rejected() {
    let tmp = repo("a\n", "b\n");
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    pump(&mut s, "diff available", |s| s.pane_diff(pane).is_some());
    for (id, p) in [
        ("preview.diff.mode", json!({"mode": "bogus"})),
        ("preview.diff.toggle", json!({"show": "yes"})),
        ("preview.diff.rev", json!({})),
        ("preview.diff.rev", json!({"rev": "--output=x"})),
        ("preview.diff.rev", json!({"rev": ""})),
        ("preview.diff.rev", json!({"rev": 5})),
        ("preview.diff.next", json!({"pane": 9999})),
    ] {
        assert!(s.dispatch(id, &p).is_err(), "{id} {p}");
    }
    assert!(
        s.dispatch("pane.kind.set", &json!({"kind": "diff"}))
            .is_err()
    );
}

#[test]
fn the_picker_lists_refs_and_choosing_one_closes_it() {
    let tmp = repo("a\n", "b\n");
    git(tmp.path(), &["branch", "feature/x"]);
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    pump(&mut s, "diff available", |s| s.pane_diff(pane).is_some());
    s.dispatch("preview.diff.picker", &json!({"open": true}))
        .unwrap();
    pump(&mut s, "refs", |s| {
        s.pane_diff(pane).is_some_and(|d| d.refs.is_some())
    });
    let refs = s.pane_diff(pane).unwrap().refs.clone().unwrap();
    assert!(refs.local.iter().any(|b| b == "feature/x"));
    s.dispatch("preview.diff.rev", &json!({"rev": "feature/x"}))
        .unwrap();
    let d = s.pane_diff(pane).unwrap();
    assert!(!d.picker);
    assert_eq!(d.rev(), "feature/x");
    assert!(
        s.dispatch("preview.diff.picker", &json!({"open": "yes"}))
            .is_err()
    );
}

#[test]
fn a_bad_revision_fails_with_a_message() {
    let tmp = repo("a\n", "b\n");
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &tmp.path().join("a.txt"));
    pump(&mut s, "diff available", |s| s.pane_diff(pane).is_some());
    s.dispatch("preview.diff.rev", &json!({"rev": "no-such-branch"}))
        .unwrap();
    pump(&mut s, "failure", |s| {
        matches!(
            s.diff_view(pane).map(|v| v.status.clone()),
            Some(DiffStatus::Failed(_))
        )
    });
}

#[test]
fn the_diff_choice_and_revision_follow_the_pane_between_files() {
    let tmp = repo("1\n2\n", "1\nX\n");
    let d = tmp.path();
    std::fs::write(d.join("b.txt"), "b\n").unwrap();
    git(d, &["add", "b.txt"]);
    git(d, &["commit", "-q", "-m", "b"]);
    git(d, &["branch", "topic"]);
    let mut s = Session::new().unwrap();
    let pane = open_in_preview(&mut s, &d.join("a.txt"));
    pump(&mut s, "diff by default", |s| {
        s.pane_diff(pane).is_some_and(|x| x.active)
    });
    s.dispatch("preview.diff.rev", &json!({ "pane": pane, "rev": "topic" }))
        .unwrap();
    s.poll();
    // b.txt is unmodified, which would normally default to the plain preview.
    let uri = fm_vfs::VfsUri::parse(&d.join("b.txt").to_string_lossy()).unwrap();
    s.dispatch("preview.open", &json!({ "uri": uri.to_string() }))
        .unwrap();
    pump(&mut s, "sticky diff on next file", |s| {
        s.pane_diff(pane)
            .is_some_and(|x| x.path.ends_with("b.txt") && x.active && x.rev() == "topic")
    });
}
