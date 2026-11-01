#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Git status in the session: cached per repository, live-refreshed, switchable.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use fm_engine::git::Mark;
use fm_engine::{Params, Session};
use fm_vfs::VfsUri;
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

fn wait(
    s: &mut Session,
    uri: &VfsUri,
    mut ok: impl FnMut(&fm_engine::git::RepoStatus) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        s.poll();
        if let Some(st) = s.git_view(uri)
            && ok(&st)
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    false
}

fn repo() -> (tempfile::TempDir, Session, VfsUri) {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    git(d, &["init", "-q", "-b", "main"]);
    std::fs::write(d.join("a.txt"), "1\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-q", "-m", "i"]);
    let uri = VfsUri::parse(&d.to_string_lossy()).unwrap();
    let mut s = Session::new().unwrap();
    s.dispatch("nav.open", &Params::String(uri.to_string()))
        .unwrap();
    (tmp, s, uri)
}

#[test]
fn status_arrives_and_follows_live_edits() {
    let (tmp, mut s, uri) = repo();
    assert!(wait(&mut s, &uri, |st| st.branch.as_deref() == Some("main")));
    std::fs::write(tmp.path().join("a.txt"), "2\n").unwrap();
    assert!(wait(&mut s, &uri, |st| st.mark("a.txt") == Some(Mark::Modified)));
    git(tmp.path(), &["add", "a.txt"]);
    git(tmp.path(), &["commit", "-q", "-m", "x"]);
    assert!(wait(&mut s, &uri, |st| st.mark("a.txt").is_none()));
}

#[test]
fn one_repo_is_shared_and_cache_is_bounded() {
    let (tmp, mut s, uri) = repo();
    assert!(wait(&mut s, &uri, |_| true));
    std::fs::create_dir(tmp.path().join("sub")).unwrap();
    let sub = VfsUri::parse(&tmp.path().join("sub").to_string_lossy()).unwrap();
    assert!(s.git_view(&sub).is_some());
    assert_eq!(s.git.cached(), 1);
    for _ in 0..12 {
        let t = tempfile::tempdir().unwrap();
        git(t.path(), &["init", "-q"]);
        let u = VfsUri::parse(&t.path().to_string_lossy()).unwrap();
        s.git_view(&u);
    }
    assert!(s.git.cached() <= fm_engine::git::MAX_REPOS);
}

#[test]
fn non_repo_and_disabled_show_nothing() {
    let (_tmp, mut s, uri) = repo();
    let plain = tempfile::tempdir().unwrap();
    let p = VfsUri::parse(&plain.path().to_string_lossy()).unwrap();
    assert!(s.git_view(&p).is_none());
    assert!(wait(&mut s, &uri, |_| true));
    s.dispatch("git.markers.toggle", &Params::Null).unwrap();
    assert!(!s.prefs.git_enabled);
    assert!(s.git_view(&uri).is_none());
    s.dispatch("git.markers.toggle", &Params::Null).unwrap();
    assert!(s.prefs.git_enabled);
}

#[test]
fn commands_reject_bad_params() {
    let (_tmp, mut s, _uri) = repo();
    for bad in [
        json!("x"),
        json!(5),
        json!({"which": 3}),
        json!({"which": "nope"}),
    ] {
        assert!(s.dispatch("git.markers.toggle", &bad).is_err(), "{bad}");
    }
    s.dispatch("git.markers.toggle", &json!({"which": "gutter"}))
        .unwrap();
    assert!(!s.prefs.git_gutter);
    s.dispatch("git.refresh", &Params::Null).unwrap();
    assert!(
        s.dispatch("prefs.set", &json!({"key": "git_tint", "value": "x"}))
            .is_err()
    );
}
