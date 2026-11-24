//! Locations inside archives survive a restart (add-archive-vfs 2.2/4.1): the handle
//! table is saved to `archives.json` and loaded before workspaces, and hostile entries
//! are dropped. One test in its own process, because the table is process-wide.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::{Cursor, Write as _};
use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::Session;
use fm_engine::archive_cmds::{REFS_FILE, load_refs, load_refs_file, refs_tick, save_refs_file};
use fm_vfs::VfsUri;

fn zip_bytes() -> Vec<u8> {
    let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
    w.start_file("www/index.html", zip::write::SimpleFileOptions::default())
        .expect("start");
    w.write_all(b"<h1>hi</h1>").expect("w");
    w.finish().expect("finish").into_inner()
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

#[test]
fn archive_locations_open_again_after_a_restart() {
    let dir = tempfile::tempdir().expect("tmp");
    let config = dir.path().join("config");
    let zip = dir.path().join("site.zip");
    std::fs::write(&zip, zip_bytes()).expect("zip");
    let zip_uri = VfsUri::parse(zip.to_str().expect("utf8")).expect("uri");

    // A hand-made table: one real entry, one whose handle names another file.
    let handle = fm_archive::handle_for(&zip_uri, 0, None);
    let other = VfsUri::parse("/etc/passwd").expect("uri");
    let junk = json!({"container": 42});
    let text = json!({
        "version": 1,
        "refs": {
            handle.clone(): {"container": zip_uri},
            "00000000000000ff": {"container": other},
            "00000000000000fe": junk,
            // Not normalised: the derived `Deserialize` would take it as is.
            fm_archive::handle_for(&zip_uri, 7, None): {"container": {"scheme": "File", "authority": {"user": null, "host": "", "port": null}, "path": "/a/../../etc"}, "offset": 7},
        }
    })
    .to_string();
    std::fs::create_dir_all(&config).expect("config");
    std::fs::write(config.join(REFS_FILE), &text).expect("refs");
    assert_eq!(load_refs_file(&config), 1, "the forged entry is dropped");
    assert_eq!(load_refs("not json"), 0);
    assert_eq!(load_refs(r#"{"version": 9, "refs": {}}"#), 0);

    // A new session (as after a restart) reads a restored location inside the archive.
    let mut s = Session::new().expect("session");
    let www = VfsUri::archive_root(&handle)
        .and_then(|r| r.join("www"))
        .expect("www");
    s.ensure_read(www.clone()).expect("read restored location");
    poll_until(&mut s, "the restored listing", |s| {
        s.dirs.state(&www).is_ready()
    });
    assert!(
        fm_engine::item_cmds::display_path(&www).ends_with("site.zip/www"),
        "{}",
        fm_engine::item_cmds::display_path(&www)
    );
    let forged = VfsUri::archive_root("00000000000000ff").expect("root");
    assert!(
        s.ensure_read(forged).is_err(),
        "unknown handles stay unknown"
    );

    // Opening another archive marks the table for saving; it is written owner-only.
    let second = dir.path().join("two.zip");
    std::fs::write(&second, zip_bytes()).expect("zip");
    s.dispatch(
        "archive.enter",
        &json!({"uri": second.to_str().expect("utf8")}),
    )
    .expect("enter");
    let saved = refs_tick().expect("changed");
    assert!(saved.contains("two.zip"), "{saved}");
    assert!(refs_tick().is_none(), "nothing new to save");
    save_refs_file(&config, &saved).expect("save");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(config.join(REFS_FILE))
            .expect("meta")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
