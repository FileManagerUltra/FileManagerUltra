#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Live refresh of local panes through the OS watcher (capability `directory-cache`).

use std::time::{Duration, Instant};

use fm_engine::{DirState, Params, Session};
use fm_vfs::VfsUri;

fn pump(s: &mut Session, limit: Duration, mut done: impl FnMut(&mut Session) -> bool) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        s.poll();
        if done(s) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn open(dir: &std::path::Path) -> (Session, VfsUri) {
    let uri = VfsUri::parse(&dir.to_string_lossy()).unwrap();
    let mut s = Session::new().unwrap();
    s.dispatch("nav.open", &Params::String(uri.to_string()))
        .expect("open");
    assert!(pump(&mut s, Duration::from_secs(10), |s| s
        .dirs
        .state(&uri)
        .is_ready()));
    assert!(pump(&mut s, Duration::from_secs(5), |s| s
        .watch
        .covers(&uri)));
    (s, uri)
}

fn names(s: &Session, uri: &VfsUri) -> Vec<String> {
    s.dirs
        .entries(uri)
        .map(|e| e.iter().map(|x| x.name.clone()).collect())
        .unwrap_or_default()
}

#[test]
fn external_create_and_delete_refresh_without_polling_delay() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, uri) = open(tmp.path());
    std::fs::write(tmp.path().join("new.txt"), b"x").unwrap();
    // Well under the 2 s fallback poll: only the watcher can deliver this.
    assert!(pump(&mut s, Duration::from_millis(1500), |s| names(
        s, &uri
    )
    .contains(&"new.txt".to_string())));
    std::fs::remove_file(tmp.path().join("new.txt")).unwrap();
    assert!(pump(&mut s, Duration::from_millis(1500), |s| !names(
        s, &uri
    )
    .contains(&"new.txt".to_string())));
}

#[test]
fn burst_of_changes_stays_bounded_and_converges() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, uri) = open(tmp.path());
    for i in 0..1500 {
        std::fs::write(tmp.path().join(format!("f{i}")), b"").unwrap();
    }
    assert!(pump(&mut s, Duration::from_secs(10), |s| names(s, &uri)
        .len()
        == 1500));
    assert!(s.watch.watched() <= fm_engine::watch::MAX_WATCHES);
}

#[test]
fn vanished_directory_shows_an_error_not_a_stale_listing() {
    let tmp = tempfile::tempdir().unwrap();
    let child = tmp.path().join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::write(child.join("a"), b"").unwrap();
    let (mut s, uri) = open(&child);
    std::fs::remove_dir_all(&child).unwrap();
    assert!(pump(&mut s, Duration::from_secs(5), |s| matches!(
        s.dirs.state(&uri),
        DirState::Failed { .. }
    )));
}

#[test]
fn leaving_a_directory_drops_its_watch() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut s, uri) = open(tmp.path());
    let other = tempfile::tempdir().unwrap();
    let other_uri = VfsUri::parse(&other.path().to_string_lossy()).unwrap();
    s.dispatch("nav.open", &Params::String(other_uri.to_string()))
        .unwrap();
    assert!(pump(&mut s, Duration::from_secs(5), |s| s
        .watch
        .covers(&other_uri)
        && !s.watch.covers(&uri)));
    assert_eq!(s.watch.watched(), 1);
}

#[test]
fn selection_survives_an_unrelated_change() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("keep.txt"), b"").unwrap();
    let (mut s, uri) = open(tmp.path());
    let pane = s.layout.focused();
    s.layout
        .state_mut(pane)
        .unwrap()
        .listing
        .toggle("keep.txt", 0);
    std::fs::write(tmp.path().join("other.txt"), b"").unwrap();
    assert!(pump(&mut s, Duration::from_millis(1500), |s| names(
        s, &uri
    )
    .len()
        == 2));
    assert!(
        s.layout
            .state(pane)
            .unwrap()
            .listing
            .is_selected("keep.txt")
    );
}
