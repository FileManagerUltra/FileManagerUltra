//! Preview panes through `Session` commands (capability `preview-pane`).
#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use serde_json::json;

use fm_engine::Session;
use fm_engine::preview_cmds::Body;
use fm_layout::{PaneId, PaneKind, PreviewSource};
use fm_preview::Preview;
use fm_vfs::VfsUri;

struct Rig {
    s: Session,
    files: PaneId,
    preview: PaneId,
    _dir: tempfile::TempDir,
}

fn pump(s: &mut Session, mut until: impl FnMut(&mut Session) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        s.poll();
        if until(s) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

fn rig(files: &[(&str, &[u8])]) -> Rig {
    let dir = tempfile::tempdir().expect("tmp");
    for (n, data) in files {
        std::fs::write(dir.path().join(n), data).expect("write");
    }
    let mut s = Session::new().expect("session");
    let uri = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
    let files_pane = s.layout.focused();
    s.layout
        .state_mut(files_pane)
        .expect("pane")
        .navigate(uri.clone());
    s.ensure_read(uri.clone()).expect("read");
    assert!(pump(&mut s, |s| s.pane_view_ready(files_pane)));
    s.dispatch("view.preview.toggle", &json!({})).expect("open");
    let preview = s
        .layout
        .panes()
        .into_iter()
        .find(|p| {
            s.layout
                .state(*p)
                .is_some_and(|st| st.kind == PaneKind::Preview)
        })
        .expect("preview pane");
    Rig {
        s,
        files: files_pane,
        preview,
        _dir: dir,
    }
}

impl Rig {
    fn cursor(&mut self, name: &str) {
        let (entries, ix) = self.s.pane_view(self.files).expect("view");
        let row = ix
            .order
            .iter()
            .position(|i| entries[*i as usize].name == name)
            .expect("entry");
        let st = self.s.layout.state_mut(self.files).expect("pane");
        st.listing.on_parent = false;
        st.listing.focused_row = row;
    }

    fn body(&self) -> &Body {
        &self.s.preview_slot(self.preview).expect("slot").body
    }

    fn wait_static(&mut self) -> std::sync::Arc<Preview> {
        let p = self.preview;
        assert!(pump(&mut self.s, |s| matches!(
            s.preview_slot(p).map(|x| &x.body),
            Some(Body::Static(_))
        )));
        match self.body() {
            Body::Static(a) => a.clone(),
            _ => panic!("static"),
        }
    }
}

#[test]
fn toggle_opens_a_following_preview_and_keeps_focus() {
    let mut r = rig(&[("a.txt", b"hello")]);
    assert_eq!(r.s.layout.focused(), r.files);
    assert_eq!(
        r.s.layout.state(r.preview).expect("st").preview_source,
        PreviewSource::Follow
    );
    r.s.dispatch("view.preview.toggle", &json!({}))
        .expect("close");
    assert_eq!(r.s.layout.panes().len(), 1);
}

#[test]
fn text_selection_renders_and_follows_the_cursor() {
    let mut r = rig(&[("a.txt", b"hello\nworld\n"), ("b.rs", b"fn main() {}\n")]);
    r.cursor("a.txt");
    let Preview::Text(t) = &*r.wait_static() else {
        panic!("text")
    };
    assert_eq!(t.lines.len(), 2);
    r.cursor("b.rs");
    assert!(pump(&mut r.s, |s| matches!(
        s.preview_slot(r.preview)
            .and_then(|x| x.subject.as_ref())
            .map(|e| e.name.as_str()),
        Some("b.rs")
    )));
    let Preview::Text(t) = &*r.wait_static() else {
        panic!("text")
    };
    assert_eq!(t.lang, "rs");
}

#[test]
fn parent_row_and_directories() {
    let mut r = rig(&[("a.txt", b"x")]);
    r.s.layout
        .state_mut(r.files)
        .expect("pane")
        .listing
        .on_parent = true;
    r.s.poll();
    assert!(matches!(r.body(), Body::Empty));
    std::fs::create_dir(r._dir.path().join("sub")).expect("dir");
    std::fs::write(r._dir.path().join("sub/f"), b"1234").expect("f");
    let uri = r.s.layout.state(r.files).expect("pane").location.clone();
    r.s.refresh_read(uri).expect("refresh");
    assert!(pump(&mut r.s, |s| s
        .pane_view(r.files)
        .is_some_and(|(e, _)| e.len() == 2)));
    r.cursor("sub");
    let Preview::Dir(d) = &*r.wait_static() else {
        panic!("dir")
    };
    assert_eq!((d.files, d.bytes), (1, 4));
}

#[test]
fn pretty_is_opt_in_and_resets_on_file_change() {
    let mut r = rig(&[
        ("a.json", br#"{"a":1,"b":[1,2]}"#),
        ("bad.json", b"{oops"),
        ("c.json", b"[1]"),
    ]);
    r.cursor("a.json");
    let Preview::Structured(d) = &*r.wait_static() else {
        panic!("structured")
    };
    assert_eq!(d.raw.lines.len(), 1, "raw by default");
    assert!(!r.s.preview_slot(r.preview).expect("slot").pretty);
    r.s.dispatch("preview.toggle_pretty", &json!({}))
        .expect("pretty");
    assert!(r.s.preview_slot(r.preview).expect("slot").pretty);
    r.s.dispatch("preview.toggle_pretty", &json!({}))
        .expect("raw again");
    r.s.dispatch("preview.toggle_pretty", &json!({}))
        .expect("pretty");
    r.cursor("c.json");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(r.preview)
            .and_then(|x| x.subject.as_ref())
            .is_some_and(|e| e.name == "c.json")
    }));
    assert!(
        !r.s.preview_slot(r.preview).expect("slot").pretty,
        "reset to raw"
    );
    r.cursor("bad.json");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(r.preview)
            .and_then(|x| x.subject.as_ref())
            .is_some_and(|e| e.name == "bad.json")
    }));
    r.wait_static();
    let err =
        r.s.dispatch("preview.toggle_pretty", &json!({}))
            .expect_err("invalid");
    assert!(err.to_string().contains("not valid JSON"));
}

fn monolog(n: usize) -> Vec<u8> {
    (0..n)
        .map(|i| {
            format!(
                "[2024-05-01 10:00:{:02}] main.{}: line {i}\n",
                i % 60,
                if i % 5 == 0 { "ERROR" } else { "INFO" }
            )
        })
        .collect::<String>()
        .into_bytes()
}

#[test]
fn log_mode_filters_and_follows() {
    let mut r = rig(&[("app.log", &monolog(20))]);
    r.cursor("app.log");
    let p = r.preview;
    assert!(pump(&mut r.s, |s| s
        .preview_slot(p)
        .is_some_and(|x| x.log().is_some())));
    r.s.dispatch("preview.log.level", &json!({"level": "error"}))
        .expect("level");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(p)
            .and_then(|x| x.log())
            .is_some_and(|l| l.filtered.as_ref().is_some_and(|f| f.len() == 4))
    }));
    assert!(
        r.s.dispatch("preview.log.search", &json!({"query": "(", "regex": true}))
            .is_ok()
    );
    assert!(pump(&mut r.s, |s| s
        .preview_slot(p)
        .and_then(|x| x.log())
        .is_some_and(|l| l.filter_error.is_some())));
    r.s.dispatch("preview.log.search", &json!({"query": ""}))
        .expect("clear");
    assert!(
        r.s.dispatch("preview.log.time_range", &json!({"from": "nonsense"}))
            .is_err()
    );
    r.s.dispatch("preview.log.level", &json!({"level": null}))
        .expect("all");
    assert!(pump(&mut r.s, |s| s
        .preview_slot(p)
        .and_then(|x| x.log())
        .is_some_and(|l| l.filtered.is_none())));

    r.s.dispatch("preview.log.follow", &json!({"on": true}))
        .expect("follow");
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(r._dir.path().join("app.log"))
        .expect("open");
    std::io::Write::write_all(&mut f, b"[2024-05-01 11:00:00] main.CRITICAL: fresh\n")
        .expect("append");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(p)
            .and_then(|x| x.log())
            .is_some_and(|l| l.doc.read().expect("lock").records.len() == 21)
    }));
    r.s.dispatch("preview.log.follow", &json!({"on": false}))
        .expect("off");
}

#[test]
fn log_marker_command_sets_and_clears() {
    let mut r = rig(&[("app.log", &monolog(5))]);
    r.cursor("app.log");
    let p = r.preview;
    assert!(pump(&mut r.s, |s| s
        .preview_slot(p)
        .is_some_and(|x| x.log().is_some())));
    r.s.dispatch("preview.log.mark", &json!({})).expect("mark");
    let marks = |s: &Session| {
        s.preview_slot(p)
            .and_then(|x| x.log())
            .map(|l| l.doc.read().expect("lock").marks.clone())
    };
    assert_eq!(marks(&r.s), Some(vec![5]));
    r.s.dispatch("preview.log.mark", &json!({"clear": true}))
        .expect("clear");
    assert_eq!(marks(&r.s), Some(vec![]));
}

#[test]
fn source_pinning_and_missing_files_pane() {
    let mut r = rig(&[("a.txt", b"x")]);
    let other =
        r.s.layout
            .split(r.files, fm_layout::Axis::Vertical, 0.5)
            .expect("split");
    r.s.layout.focus_pane(r.files).expect("focus");
    r.s.dispatch("pane.preview.source", &json!({"source": other}))
        .expect("pin");
    assert_eq!(r.s.layout.preview_target(r.preview), Some(other));
    assert!(
        r.s.dispatch("pane.preview.source", &json!({"source": r.preview}))
            .is_err()
    );
    r.s.dispatch("pane.preview.source", &json!({"source": "focus"}))
        .expect("follow");
    assert_eq!(
        r.s.layout.state(r.preview).expect("st").preview_source,
        PreviewSource::Follow
    );
}

#[test]
fn preview_commands_without_a_preview_pane_fail_gracefully() {
    let mut s = Session::new().expect("session");
    for id in [
        "preview.zoom",
        "preview.toggle_wrap",
        "preview.toggle_pretty",
        "preview.log.follow",
        "preview.log.mark",
        "preview.log.earlier",
    ] {
        assert!(!s.command_enabled(id), "{id}");
        assert!(s.dispatch(id, &json!({"mode": "in"})).is_err(), "{id}");
    }
}

#[test]
fn hostile_files_become_cards_not_crashes() {
    let mut big = vec![b'{'; 5000];
    big.extend(vec![b'['; 5000]);
    let mut r = rig(&[
        ("deep.json", &big),
        ("junk.png", b"\x89PNG\r\n\x1a\nxxxx"),
        ("blob.bin", &[0u8; 64]),
    ]);
    for name in ["deep.json", "junk.png", "blob.bin"] {
        r.cursor(name);
        assert!(pump(&mut r.s, |s| {
            s.preview_slot(r.preview)
                .and_then(|x| x.subject.as_ref())
                .is_some_and(|e| e.name == name)
        }));
        r.wait_static();
    }
}
