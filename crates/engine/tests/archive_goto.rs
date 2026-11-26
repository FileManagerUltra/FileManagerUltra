//! Typing a path through archives in the address bar (add-archive-vfs 2.3, `path-display`
//! "typed into the address bar, such a form SHALL be resolved from the left").
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write as _};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::Session;
use fm_engine::archive_cmds::{container_of, nesting};
use fm_engine::item_cmds::display_path;
use fm_vfs::{Scheme, VfsUri};

fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let o =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in entries {
        if let Some(dir) = name.strip_suffix('/') {
            w.add_directory(dir, o).expect("dir");
        } else {
            w.start_file(*name, o).expect("f");
            w.write_all(data).expect("w");
        }
    }
    w.finish().expect("finish").into_inner()
}

fn uri(p: &Path) -> VfsUri {
    VfsUri::parse(p.to_str().expect("utf8")).expect("uri")
}

fn shown(p: &Path) -> String {
    display_path(&uri(p))
}

fn location(s: &Session) -> VfsUri {
    s.layout
        .state(s.layout.focused())
        .expect("pane")
        .location
        .clone()
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

/// Type `text` into the focused pane's address bar and press Enter.
fn type_path(s: &mut Session, text: &str) {
    s.dispatch("nav.goto", &json!(null)).expect("goto");
    s.dispatch("nav.goto.commit", &json!({ "path": text }))
        .unwrap_or_else(|e| panic!("commit {text}: {e}"));
    assert!(s.path_edit.is_none(), "{text} was accepted");
}

/// `dir/site.zip` with `www/index.html`, and `dir/outer.zip` holding `inner.zip` with `docs/`.
fn fixture(dir: &Path) {
    std::fs::write(
        dir.join("site.zip"),
        zip_with(&[("www/", b""), ("www/index.html", b"<p>")]),
    )
    .expect("site.zip");
    let inner = zip_with(&[("docs/", b""), ("docs/a.txt", b"a")]);
    std::fs::write(dir.join("outer.zip"), zip_with(&[("inner.zip", &inner)])).expect("outer");
}

#[test]
fn a_typed_path_through_an_archive_opens_the_folder_inside_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let mut s = Session::new().expect("session");
    let sep = std::path::MAIN_SEPARATOR;

    type_path(
        &mut s,
        &format!("{}{sep}www", shown(&tmp.path().join("site.zip"))),
    );
    let at = location(&s);
    assert_eq!(at.scheme(), Scheme::Archive);
    assert_eq!(at.path_str(), "/www");
    assert_eq!(
        container_of(&at).expect("container").container,
        uri(&tmp.path().join("site.zip"))
    );
    poll_until(&mut s, "listing", |s| s.dirs.state(&at).is_ready());
    let names: Vec<String> = s
        .dirs
        .entries(&at)
        .expect("entries")
        .iter()
        .map(|e| e.name.clone())
        .collect();
    assert_eq!(names, vec!["index.html"]);

    // The archive file alone (with or without a trailing separator) opens its root.
    for text in [
        shown(&tmp.path().join("site.zip")),
        format!("{}{sep}", shown(&tmp.path().join("site.zip"))),
    ] {
        type_path(&mut s, &text);
        let at = location(&s);
        assert_eq!(
            (at.scheme(), at.path_str()),
            (Scheme::Archive, "/"),
            "{text}"
        );
    }
}

#[test]
fn nested_archives_and_relative_paths_resolve_from_the_left() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let mut s = Session::new().expect("session");
    let sep = std::path::MAIN_SEPARATOR;

    let outer = shown(&tmp.path().join("outer.zip"));
    type_path(&mut s, &format!("{outer}{sep}inner.zip{sep}docs"));
    let at = location(&s);
    assert_eq!(nesting(&at), 2, "inside inner.zip inside outer.zip");
    assert_eq!(at.path_str(), "/docs");
    poll_until(&mut s, "nested listing", |s| s.dirs.state(&at).is_ready());

    // Relative to a plain folder.
    type_path(&mut s, &shown(tmp.path()));
    type_path(&mut s, &format!("site.zip{sep}www"));
    let at = location(&s);
    assert_eq!((nesting(&at), at.path_str()), (1, "/www"));
}

#[test]
fn every_location_inside_archives_resolves_back_from_its_display_form() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let mut s = Session::new().expect("session");
    let sep = std::path::MAIN_SEPARATOR;
    let outer = shown(&tmp.path().join("outer.zip"));
    type_path(&mut s, &format!("{outer}{sep}inner.zip{sep}docs"));
    let deep = location(&s);
    let site = shown(&tmp.path().join("site.zip"));
    type_path(&mut s, &format!("{site}{sep}www"));
    let www = location(&s);

    assert_eq!(
        (nesting(&deep), nesting(&www)),
        (2, 1),
        "both are inside archives"
    );
    // A fresh session (nothing in the history) resolves each display form back.
    let mut fresh = Session::new().expect("session");
    for target in [&deep, &www] {
        let text = display_path(target);
        type_path(&mut fresh, &text);
        assert_eq!(&location(&fresh), target, "{text}");
    }
}

#[test]
fn a_real_folder_named_like_an_archive_stays_a_folder() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::create_dir_all(tmp.path().join("photos.zip").join("2026")).expect("folder");
    let mut s = Session::new().expect("session");
    // With the parent listed, the folder is known to be one.
    type_path(&mut s, &shown(tmp.path()));
    let parent = location(&s);
    poll_until(&mut s, "listing", |s| s.dirs.state(&parent).is_ready());
    let sep = std::path::MAIN_SEPARATOR;
    type_path(&mut s, &format!("photos.zip{sep}2026"));
    assert_eq!(
        location(&s),
        uri(&tmp.path().join("photos.zip").join("2026"))
    );
}

#[test]
fn completion_continues_inside_a_typed_archive() {
    let tmp = tempfile::tempdir().expect("tmp");
    fixture(tmp.path());
    let mut s = Session::new().expect("session");
    type_path(&mut s, &shown(tmp.path()));
    let folder = location(&s);
    poll_until(&mut s, "listing", |s| s.dirs.state(&folder).is_ready());
    let sep = std::path::MAIN_SEPARATOR;
    let typed_dir = format!("{}{sep}", shown(tmp.path()));

    let suggestions = |s: &mut Session, text: &str| -> Vec<String> {
        s.dispatch("nav.goto", &json!({ "text": text }))
            .expect("goto");
        poll_until(s, "suggestions", |s| {
            s.refresh_suggestions();
            s.path_edit.as_ref().is_some_and(|e| e.is_complete())
        });
        let out = s
            .path_edit
            .as_ref()
            .map(|e| e.suggestions.iter().map(|x| x.text.clone()).collect())
            .unwrap_or_default();
        s.dispatch("nav.goto.cancel", &json!(null)).expect("cancel");
        out
    };

    // Archive files are offered like folders …
    let here = suggestions(&mut s, &format!("{typed_dir}si"));
    assert!(
        here.contains(&format!("{typed_dir}site.zip{sep}")),
        "{here:?}"
    );
    // … and completion goes on inside them.
    let inside = suggestions(&mut s, &format!("{typed_dir}site.zip{sep}"));
    assert!(
        inside.contains(&format!("{typed_dir}site.zip{sep}www{sep}")),
        "{inside:?}"
    );
}
