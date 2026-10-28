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
    fn select(&mut self, names: &[&str]) {
        self.s
            .layout
            .state_mut(self.files)
            .expect("pane")
            .listing
            .clear_selection();
        for name in names {
            let (entries, ix) = self.s.pane_view(self.files).expect("view");
            let row = ix
                .order
                .iter()
                .position(|i| entries[*i as usize].name == *name)
                .expect("row");
            self.s
                .layout
                .state_mut(self.files)
                .expect("pane")
                .listing
                .toggle(name, row);
        }
    }

    fn cursor(&mut self, name: &str) {
        let (entries, ix) = self.s.pane_view(self.files).expect("view");
        let row = ix
            .order
            .iter()
            .position(|i| entries[*i as usize].name == name)
            .expect("entry");
        let st = self.s.layout.state_mut(self.files).expect("pane");
        st.listing.on_current = false;
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
fn filename_filters_reclassify_loaded_single_and_multi_previews() {
    let mut r = rig(&[("a.trace", b"alpha\n"), ("b.trace", b"beta\n")]);
    r.cursor("a.trace");
    assert!(matches!(&*r.wait_static(), Preview::Text(_)));
    r.s.dispatch(
        "prefs.preview.filters",
        &json!({"groups": [{"type": "logs", "patterns": ["*.trace"]}]}),
    )
    .expect("associate");
    let pane = r.preview;
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|slot| slot.log())
        .is_some()));
    r.select(&["a.trace", "b.trace"]);
    wait_multi(&mut r, 2);
    r.s.dispatch(
        "prefs.preview.filters",
        &json!({"groups": [{"type": "logs", "patterns": []}, {"type": "images", "patterns": ["*.trace"]}]}),
    )
    .expect("change");
    assert!(pump(&mut r.s, |s| matches!(
        s.preview_slot(pane).map(|slot| &slot.body),
        Some(Body::Grid(_))
    )));
    r.s.dispatch(
        "prefs.preview.filters",
        &json!({"groups": [{"type": "markdown", "patterns": ["*.trace"]}, {"type": "images", "patterns": []}]}),
    )
    .expect("markdown");
    r.select(&[]);
    r.cursor("a.trace");
    assert!(pump(
        &mut r.s,
        |s| matches!(s.preview_slot(pane).map(|slot| &slot.body),
        Some(Body::Static(preview)) if matches!(&**preview, Preview::Markdown(_)))
    ));
    r.s.dispatch("prefs.preview.filters", &json!({"reset": true}))
        .expect("reset");
    assert!(pump(
        &mut r.s,
        |s| matches!(s.preview_slot(pane).map(|slot| &slot.body),
        Some(Body::Static(preview)) if matches!(&**preview, Preview::Text(_)))
    ));
}

#[test]
fn custom_structured_text_and_media_filters_reach_workers() {
    let mut r = rig(&[
        ("a.custom", b"{\"message\":\"hello\"}"),
        ("b.custom", b"<html>hello</html>"),
        ("c.custom", b"first,second\none,two\n"),
        ("d.custom", b"%PDF-1.7\n"),
    ]);
    r.s.dispatch(
        "prefs.preview.filters",
        &json!({"groups": [
            {"type": "json", "patterns": ["a.custom"]},
            {"type": "html", "patterns": ["b.custom"]},
            {"type": "csv", "patterns": ["c.custom"]}
        ]}),
    )
    .expect("custom filters");
    r.cursor("a.custom");
    assert!(
        matches!(&*r.wait_static(), Preview::Structured(doc) if doc.format == fm_preview::Format::Json)
    );
    for (name, lang) in [("b.custom", "html"), ("c.custom", "csv")] {
        r.cursor(name);
        let pane = r.preview;
        assert!(pump(
            &mut r.s,
            |s| matches!(s.preview_slot(pane).map(|slot| &slot.body),
            Some(Body::Static(preview)) if matches!(&**preview, Preview::Text(doc) if doc.lang == lang))
        ));
    }
    r.cursor("d.custom");
    let pane = r.preview;
    assert!(pump(
        &mut r.s,
        |s| matches!(s.preview_slot(pane).map(|slot| &slot.body),
        Some(Body::Static(preview)) if matches!(&**preview, Preview::Meta(doc) if doc.hint.as_deref().is_some_and(|hint| hint.contains("PDF"))))
    ));
}

fn wait_multi(rig: &mut Rig, sources: usize) {
    let pane = rig.preview;
    assert!(pump(&mut rig.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(|v| v.sources.len() == sources)));
}

fn visible_text(s: &Session, pane: PaneId) -> Vec<String> {
    let view = s.preview_slot(pane).expect("slot").log().expect("log");
    let doc = view.doc.read().expect("doc");
    let indices: Vec<_> = view
        .filtered
        .as_ref()
        .map(|f| f.as_ref().clone())
        .unwrap_or_else(|| (0..doc.records.len() as u32).collect());
    indices
        .into_iter()
        .map(|i| doc.records[i as usize].text.clone())
        .collect()
}

#[test]
fn selected_logs_share_filters_and_both_orders_without_losing_sources() {
    let mut r = rig(&[
        (
            "a.log",
            b"2024-05-01 11:00:00 ERROR alpha\n2024-05-01 12:00:00 INFO same-a\nunknown alpha\n",
        ),
        (
            "b.log",
            b"2024-05-01 10:00:00 ERROR beta\n2024-05-01 12:00:00 INFO same-b\nunknown beta\n",
        ),
        ("c.log", b"2024-05-01 09:00:00 ERROR gamma\n"),
    ]);
    r.select(&["b.log", "a.log"]);
    wait_multi(&mut r, 2);
    let pane = r.preview;
    let lines = visible_text(&r.s, pane);
    assert!(lines[0].contains("alpha"));
    assert!(
        lines[3].contains("beta"),
        "concatenate in display order, not click order"
    );
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert_eq!(
        view.sources
            .iter()
            .map(|s| s.entry.name.as_str())
            .collect::<Vec<_>>(),
        ["a.log", "b.log"]
    );
    assert_eq!(view.sources[0].records, 0..3);
    assert_eq!(view.sources[1].records, 3..6);
    for bad in [
        json!({"mode": null}),
        json!({"mode": "bad"}),
        json!({"mode": 1}),
    ] {
        assert!(r.s.dispatch("preview.log.order", &bad).is_err());
    }
    r.s.dispatch("preview.log.order", &json!({}))
        .expect("order");
    assert!(pump(&mut r.s, |s| visible_text(s, pane)[0].contains("beta")));
    let ordered = visible_text(&r.s, pane);
    assert!(ordered[2].contains("same-a"));
    assert!(ordered[3].contains("same-b"), "equal timestamps are stable");
    assert_eq!(&ordered[4..], &["unknown alpha", "unknown beta"]);
    r.s.dispatch(
        "preview.log.search",
        &json!({"query": "alpha|beta", "regex": true}),
    )
    .expect("regex");
    r.s.dispatch("preview.log.level", &json!({"level": "error"}))
        .expect("severity");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 2));
    r.s.dispatch(
        "preview.log.time_range",
        &json!({"from": "2024-05-01 10:30:00"}),
    )
    .expect("time");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 1));
    assert!(visible_text(&r.s, pane)[0].contains("alpha"));
    r.s.dispatch("preview.log.order", &json!({"mode": "concatenate"}))
        .expect("concat");
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(|v| !v.chronological)));
    assert_eq!(visible_text(&r.s, pane).len(), 1);
    assert!(
        r.s.dispatch("preview.log.follow", &json!({"on": true}))
            .is_err()
    );
    r.select(&["a.log", "c.log"]);
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(|v| v
            .sources
            .get(1)
            .is_some_and(|s| s.entry.name == "c.log"))));
    assert_eq!(
        r.s.preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .filter
            .query,
        "alpha|beta"
    );
    r.select(&["c.log"]);
    assert!(pump(&mut r.s, |s| s.preview_slot(pane).is_some_and(
        |slot| slot.subjects.len() == 1 && slot.log().is_some_and(|v| v.sources.is_empty())
    )));
}

#[test]
fn clearing_filters_restores_records_and_preserves_order_and_display_settings() {
    let mut r = rig(&[
        ("a.log", b"2024-05-01 11:00:00 ERROR alpha\n"),
        ("b.log", b"2024-05-01 10:00:00 INFO beta\n"),
    ]);
    r.select(&["a.log", "b.log"]);
    wait_multi(&mut r, 2);
    let pane = r.preview;
    for (command, params) in [
        ("preview.log.order", json!({"mode": "chronological"})),
        ("preview.log.fold", json!({"mode": "expand"})),
        (
            "preview.log.search",
            json!({"query": "alpha", "regex": true}),
        ),
        ("preview.log.level", json!({"level": "error"})),
        (
            "preview.log.time_range",
            json!({"from": "2024-05-01 10:30:00", "to": "2024-05-01 12:00:00"}),
        ),
    ] {
        r.s.dispatch(command, &params).expect("configure");
    }
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 1));
    for bad in [
        json!([]),
        json!({"pane": "bad"}),
        json!({"pane": -1}),
        json!({"pane": 9999}),
    ] {
        assert!(r.s.dispatch("preview.log.clear_filters", &bad).is_err());
    }
    r.s.dispatch("preview.log.clear_filters", &json!({"pane": pane}))
        .expect("clear");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 2));
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert!(view.filter.is_empty());
    assert!(view.filter_error.is_none());
    assert!(view.chronological && view.expand_all && view.columns);
    assert!(visible_text(&r.s, pane)[0].contains("beta"));
    r.s.dispatch("preview.log.search", &json!({"query": "[", "regex": true}))
        .expect("bad regex");
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(|v| v.filter_error.is_some())));
    r.s.dispatch("preview.log.clear_filters", &json!({"pane": pane}))
        .expect("clear error");
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(
            |v| v.filter.is_empty() && v.filter_error.is_none()
        )));
}

#[test]
fn selected_files_stay_loaded_until_explicit_reload_after_changes() {
    let mut r = rig(&[
        ("a.log", b"2024-05-01 11:00:00 ERROR alpha\n"),
        ("b.log", b"2024-05-01 10:00:00 INFO beta-longer\n"),
    ]);
    r.s.layout.focus_pane(r.files).expect("files");
    r.s.dispatch("view.sort.size", &json!({}))
        .expect("sort by size");
    r.select(&["a.log", "b.log"]);
    wait_multi(&mut r, 2);
    let pane = r.preview;
    r.s.dispatch("preview.log.order", &json!({"mode": "chronological"}))
        .expect("order");
    r.s.dispatch("preview.log.search", &json!({"query": "alpha"}))
        .expect("search");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 1));
    let doc =
        r.s.preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .doc
            .clone();
    std::fs::write(
        r._dir.path().join("a.log"),
        b"2024-05-01 11:00:00 ERROR alpha\n2024-05-01 12:00:00 ERROR alpha-new\n",
    )
    .expect("append");
    let location = r.s.layout.state(r.files).expect("pane").location.clone();
    r.s.refresh_read(location).expect("refresh listing");
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .is_some_and(|slot| slot.updates_available)));
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert_eq!(
        view.sources[0].entry.name, "a.log",
        "metadata-driven sorting does not reload"
    );
    assert!(
        std::sync::Arc::ptr_eq(&doc, &view.doc),
        "keep the loaded document"
    );
    assert_eq!(visible_text(&r.s, pane).len(), 1);
    for bad in [
        json!([]),
        json!({"pane": "bad"}),
        json!({"pane": -1}),
        json!({"pane": 9999}),
    ] {
        assert!(r.s.dispatch("preview.reload", &bad).is_err());
    }
    r.s.dispatch("preview.reload", &json!({"pane": pane}))
        .expect("reload");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).len() == 2));
    let slot = r.s.preview_slot(pane).expect("slot");
    assert!(!slot.updates_available);
    let view = slot.log().expect("log");
    assert_eq!(
        view.sources[0].entry.name, "b.log",
        "explicit reload uses current listing order"
    );
    assert!(view.chronological);
    assert_eq!(view.filter.query, "alpha");
    assert!(!std::sync::Arc::ptr_eq(&doc, &view.doc));
    r.select(&["b.log"]);
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .is_some_and(|slot| slot.subjects.len() == 1)));
    assert!(
        r.s.dispatch("preview.reload", &json!({"pane": pane}))
            .is_err()
    );
}

#[test]
fn selection_changes_even_with_an_unchanged_cursor_and_sort_order() {
    let mut r = rig(&[
        ("a.txt", b"alpha\n"),
        ("b.txt", b"beta\n"),
        ("c.txt", b"gamma\n"),
    ]);
    r.select(&["a.txt", "b.txt"]);
    wait_multi(&mut r, 2);
    let pane = r.preview;
    let cursor = r.s.layout.state(r.files).expect("pane").listing.focused_row;
    r.s.layout
        .state_mut(r.files)
        .expect("pane")
        .listing
        .toggle("c.txt", cursor);
    wait_multi(&mut r, 3);
    assert_eq!(
        r.s.layout.state(r.files).expect("pane").listing.focused_row,
        cursor
    );
    r.s.layout
        .state_mut(r.files)
        .expect("pane")
        .listing
        .sort
        .dir = fm_layout::SortDir::Desc;
    r.s.invalidate_view(r.files);
    assert!(pump(&mut r.s, |s| s
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_some_and(|v| v
            .sources
            .first()
            .is_some_and(|source| source.entry.name == "c.txt"))));
    r.s.dispatch("preview.log.search", &json!({"query": "beta"}))
        .expect("search");
    assert!(pump(&mut r.s, |s| visible_text(s, pane) == ["beta"]));
    r.s.layout
        .state_mut(r.files)
        .expect("pane")
        .listing
        .clear_selection();
    r.cursor("a.txt");
    assert!(pump(&mut r.s, |s| s.preview_slot(pane).is_some_and(
        |slot| slot.subjects.len() == 1 && matches!(slot.body, Body::Static(_))
    )));
}

#[test]
fn gzip_rotations_and_missing_sources_are_not_silently_dropped() {
    use std::io::Write;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(b"update-alternatives 2024-05-01 09:00:00: ERROR beta\n")
        .expect("compress");
    let compressed = gz.finish().expect("gz");
    let mut r = rig(&[
        ("a.log", b"2024-05-01 10:00:00 ERROR alpha\n"),
        ("b.log.1.gz", &compressed),
        ("gone.log", b"x"),
    ]);
    std::fs::remove_file(r._dir.path().join("gone.log")).expect("remove");
    r.select(&["a.log", "b.log.1.gz", "gone.log"]);
    wait_multi(&mut r, 3);
    let pane = r.preview;
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert!(
        view.sources[2]
            .note
            .as_ref()
            .expect("error")
            .contains("Cannot preview:")
    );
    assert_eq!(view.doc.read().expect("doc").records.len(), 2);
    r.s.dispatch("preview.log.order", &json!({"mode": "chronological"}))
        .expect("sort");
    assert!(pump(&mut r.s, |s| visible_text(s, pane)[0].contains("beta")));
}

#[test]
fn images_and_mixed_formats_use_cards_and_source_count_is_bounded() {
    let mut r = rig(&[("a.txt", b"text"), ("b.bin", &[0, 1, 2, 3])]);
    r.select(&["a.txt", "b.bin"]);
    let pane = r.preview;
    assert!(pump(
        &mut r.s,
        |s| matches!(s.preview_slot(pane).map(|s| &s.body), Some(Body::Grid(items)) if items.len() == 2)
    ));
    assert!(!r.s.command_enabled("preview.log.order"));
    let files: Vec<_> = (0..=fm_engine::preview_multi::MAX_SOURCES)
        .map(|i| (format!("{i:02}.txt"), vec![b'x']))
        .collect();
    let refs: Vec<_> = files
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let mut r = rig(&refs);
    let names: Vec<_> = refs.iter().map(|(n, _)| *n).collect();
    r.select(&names);
    let pane = r.preview;
    assert!(pump(
        &mut r.s,
        |s| matches!(s.preview_slot(pane).map(|s| &s.body), Some(Body::Static(p)) if matches!(&**p, Preview::Error(e) if e.contains("at most 64")))
    ));
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
fn the_current_folder_row_previews_the_folder_itself() {
    let mut r = rig(&[("a.txt", b"x")]);
    let st = r.s.layout.state(r.files).expect("pane");
    assert!(st.listing.on_current, "navigating starts on `.`");
    let Preview::Dir(d) = &*r.wait_static() else {
        panic!("dir")
    };
    assert_eq!((d.files, d.bytes), (1, 1));
    let location = r.s.layout.state(r.files).expect("pane").location.clone();
    let subject =
        r.s.preview_slot(r.preview)
            .and_then(|x| x.subject.as_ref().map(|e| (e.name.clone(), e.uri.clone())));
    assert_eq!(subject, Some((".".to_string(), location)));
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
fn configured_initial_tail_controls_single_and_combined_log_windows() {
    let data: String = (0..24_000)
        .map(|i| {
            format!(
                "[2024-05-01 10:00:00] main.INFO: record {i:06} {}\n",
                "x".repeat(80)
            )
        })
        .collect();
    assert!(data.len() > 2 << 20);
    for (tail, multi) in [(1, false), (0, false), (1, true), (0, true)] {
        let mut r = rig(&[("a.log", data.as_bytes()), ("b.log", data.as_bytes())]);
        r.s.dispatch("prefs.preview.log", &json!({"tail_mib": tail}))
            .expect("setting");
        if multi {
            r.select(&["a.log", "b.log"]);
            wait_multi(&mut r, 2);
        } else {
            r.cursor("a.log");
            let pane = r.preview;
            assert!(pump(&mut r.s, |s| s
                .preview_slot(pane)
                .and_then(|slot| slot.log())
                .is_some()));
        }
        let v =
            r.s.preview_slot(r.preview)
                .expect("slot")
                .log()
                .expect("log");
        let d = v.doc.read().expect("doc");
        if tail == 0 {
            assert!(!d.more_before, "unlimited loads these small files fully");
            assert_eq!(d.records.len(), 24_000 * if multi { 2 } else { 1 });
        } else {
            assert!(d.more_before);
            assert!(
                d.records
                    .iter()
                    .map(|record| record.text.len())
                    .sum::<usize>()
                    <= (1 << 20) + 4096
            );
        }
    }
}

#[test]
fn combined_load_earlier_expands_all_sources_and_preserves_filters() {
    let payload: String = (0..30_000)
        .map(|i| {
            format!(
                "2024-05-01 10:00:00 INFO record{i:05} {}\n",
                "x".repeat(180)
            )
        })
        .collect();
    assert!(payload.len() > 4 << 20 && payload.len() < 8 << 20);
    let mut r = rig(&[("a.log", payload.as_bytes()), ("b.log", payload.as_bytes())]);
    r.select(&["a.log", "b.log"]);
    wait_multi(&mut r, 2);
    let pane = r.preview;
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert!(view.doc.read().expect("doc").more_before);
    assert!(view.doc.read().expect("doc").records.len() < 60_000);
    r.s.dispatch("preview.log.search", &json!({"query": "record00000"}))
        .expect("search");
    assert!(pump(&mut r.s, |s| visible_text(s, pane).is_empty()));
    r.s.dispatch("preview.log.order", &json!({"mode": "chronological"}))
        .expect("order");
    r.s.dispatch("preview.log.earlier", &json!({}))
        .expect("earlier");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(pane).and_then(|s| s.log()).is_some_and(|v| {
            v.window_bytes == 16 << 20
                && !v.doc.read().expect("doc").more_before
                && v.filtered.as_ref().is_some_and(|f| f.len() == 2)
        })
    }));
    let view = r.s.preview_slot(pane).expect("slot").log().expect("log");
    assert!(view.chronological);
    assert_eq!(view.filter.query, "record00000");
    let doc = view.doc.read().expect("doc");
    assert_eq!(doc.records.len(), 60_000);
    assert_ne!(
        view.record_key(0, &doc.records[0]),
        view.record_key(30_000, &doc.records[30_000])
    );
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
        "preview.log.order",
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

#[test]
fn thumbnail_mode_requests_only_the_visible_window() {
    use fm_engine::thumbs::ThumbState;
    let dir = tempfile::tempdir().expect("tmp");
    let tiny = |p: std::path::PathBuf| {
        image::RgbaImage::from_pixel(32, 16, image::Rgba([1, 2, 3, 255]))
            .save(p)
            .expect("png");
    };
    std::fs::create_dir(dir.path().join("photos")).expect("dir");
    for i in 0..40 {
        tiny(dir.path().join(format!("photos/{i:02}.png")));
    }
    std::fs::write(dir.path().join("photos/notes.txt"), b"x").expect("txt");
    let mut r = rig(&[]);
    let uri = VfsUri::parse(&dir.path().to_string_lossy()).expect("uri");
    r.s.layout
        .state_mut(r.files)
        .expect("pane")
        .navigate(uri.clone());
    r.s.refresh_read(uri).expect("read");
    assert!(pump(&mut r.s, |s| s
        .pane_view(r.files)
        .is_some_and(|(e, _)| e.len() == 1)));
    r.cursor("photos");
    let Preview::Dir(d) = &*r.wait_static() else {
        panic!("dir")
    };
    assert_eq!(d.items.len(), 41);

    r.s.dispatch("preview.thumbs.toggle", &json!({}))
        .expect("on");
    r.s.dispatch("preview.thumbs.visible", &json!({"first": 0, "count": 5}))
        .expect("visible");
    let items = d.items.clone();
    assert!(pump(&mut r.s, |s| {
        (0..5).all(|i| matches!(s.thumbs.get(&items[i], 160), ThumbState::Ready(_)))
    }));
    // Beyond one screen ahead nothing was queued.
    assert!(matches!(r.s.thumbs.get(&items[30], 160), ThumbState::None));
    // Text files get a tile, never a request.
    assert!(matches!(r.s.thumbs.get(&items[40], 160), ThumbState::None));

    r.s.dispatch("preview.thumbs.visible", &json!({"first": 30, "count": 5}))
        .expect("scrolled");
    assert!(pump(&mut r.s, |s| {
        (30..35).all(|i| matches!(s.thumbs.get(&items[i], 160), ThumbState::Ready(_)))
    }));

    r.s.dispatch("preview.thumbs.size", &json!({"size": "large"}))
        .expect("size");
    assert!(pump(&mut r.s, |s| {
        (30..35).all(|i| matches!(s.thumbs.get(&items[i], 512), ThumbState::Ready(_)))
    }));
    assert!(
        r.s.dispatch("preview.thumbs.size", &json!({"size": "huge"}))
            .is_err()
    );
    assert!(
        r.s.dispatch("preview.thumbs.visible", &json!({"first": -1, "count": 5}))
            .is_err()
    );
    assert!(
        r.s.dispatch(
            "preview.thumbs.visible",
            &json!({"first": 0, "count": 999999})
        )
        .is_err()
    );
}

#[test]
fn a_video_plays_pauses_and_seeks_in_the_preview_pane() {
    let Some(ffmpeg) = fm_preview::video::find_tool("ffmpeg") else {
        return;
    };
    let dir = tempfile::tempdir().expect("tmp");
    let clip = dir.path().join("clip.mp4");
    let made = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg("testsrc=size=320x240:rate=25:duration=4")
        .args(["-pix_fmt", "yuv420p", "-y"])
        .arg(&clip)
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return;
    }
    let bytes = std::fs::read(&clip).expect("clip");
    let mut r = rig(&[("clip.mp4", &bytes)]);
    r.cursor("clip.mp4");
    let Preview::Meta(m) = &*r.wait_static() else {
        panic!("video card")
    };
    assert!(m.video.is_some(), "ffprobe gave playback info");

    let state = |r: &Rig| {
        r.s.preview_slot(r.preview)
            .and_then(|sl| sl.video.as_ref())
            .map(|v| (v.playing, v.pos, v.frame.is_some()))
    };
    // Nothing starts before the UI reports a width.
    assert!(pump(&mut r.s, |s| s
        .preview_slot(r.preview)
        .is_some_and(|sl| sl.video.is_some())));
    assert_eq!(state(&r).map(|t| t.2), Some(false));
    r.s.dispatch(
        "preview.video.fit",
        &json!({"width": 300, "pane": r.preview}),
    )
    .expect("fit");
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(r.preview)
            .and_then(|sl| sl.video.as_ref())
            .is_some_and(|v| v.frame.is_some())
    }));
    let width = r.s.preview_slot(r.preview).and_then(|sl| sl.video.as_ref());
    assert_eq!(
        width.and_then(|v| v.frame.as_ref()).map(|f| f.width),
        Some(300_u32.div_ceil(64) * 64).map(|w| w.min(320))
    );

    r.s.dispatch("preview.video.play", &json!({"on": false}))
        .expect("pause");
    let paused = state(&r).expect("state");
    assert!(!paused.0);
    r.s.dispatch("preview.video.seek", &json!({"to": 2.0}))
        .expect("seek");
    // A paused seek decodes one frame at the new position and stays paused.
    assert!(pump(&mut r.s, |s| {
        s.preview_slot(r.preview)
            .and_then(|sl| sl.video.as_ref())
            .is_some_and(|v| v.player_is_idle() && v.frame.is_some() && (v.pos - 2.0).abs() < 0.01)
    }));
    assert!(!state(&r).expect("state").0);

    // Bad params are rejected and change nothing.
    assert!(
        r.s.dispatch("preview.video.seek", &json!({"to": "x"}))
            .is_err()
    );
    assert!(
        r.s.dispatch("preview.video.fit", &json!({"width": 0}))
            .is_err()
    );

    // Playing on to the end stops by itself.
    r.s.dispatch("preview.video.play", &json!({"on": true}))
        .expect("play");
    r.s.dispatch("preview.video.seek", &json!({"to": 3.9}))
        .expect("seek to the end");
    assert!(
        pump(&mut r.s, |s| {
            s.preview_slot(r.preview)
                .and_then(|sl| sl.video.as_ref())
                .is_some_and(|v| !v.playing)
        }),
        "playback ends at the last frame"
    );
}

#[test]
fn a_folder_preview_refreshes_after_a_file_action_inside_it() {
    let mut r = rig(&[]);
    let sub = r._dir.path().join("sub");
    std::fs::create_dir(&sub).expect("sub");
    std::fs::write(sub.join("x.txt"), b"x").expect("x");
    let sub_uri = VfsUri::parse(&sub.to_string_lossy()).expect("uri");
    r.s.refresh_read(r.s.layout.state(r.files).expect("pane").location.clone())
        .expect("refresh");
    let files = r.files;
    assert!(pump(&mut r.s, |s| s.pane_view_ready(files)));
    r.cursor("sub");
    let items = |p: &Preview| match p {
        Preview::Dir(d) => d.items.len(),
        _ => 0,
    };
    assert_eq!(items(&r.wait_static()), 1);
    r.s.dispatch(
        "file.mkdir",
        &json!({"parent": sub_uri.to_string(), "name": "z"}),
    )
    .expect("mkdir");
    let p = r.preview;
    assert!(pump(&mut r.s, |s| matches!(
        s.preview_slot(p).map(|x| &x.body),
        Some(Body::Static(a)) if items(a) == 2
    )));
}

#[test]
fn opening_a_folder_from_a_focused_preview_navigates_the_files_pane() {
    let mut r = rig(&[]);
    let sub = r._dir.path().join("sub");
    std::fs::create_dir(&sub).expect("sub");
    let sub_uri = VfsUri::parse(&sub.to_string_lossy()).expect("uri");
    r.s.layout.focus_pane(r.preview).expect("focus");
    r.s.dispatch("nav.open", &json!(sub_uri.to_string()))
        .expect("open");
    assert_eq!(r.s.layout.state(r.files).expect("files").location, sub_uri);
}

#[test]
fn new_images_in_a_shown_grid_get_thumbnails_without_a_window_change() {
    use fm_engine::thumbs::ThumbState;
    let mut r = rig(&[]);
    let sub = r._dir.path().join("sub");
    std::fs::create_dir(&sub).expect("sub");
    let png = |name: &str| {
        image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]))
            .save(sub.join(name))
            .expect("png");
    };
    png("a.png");
    let files = r.files;
    r.s.refresh_read(r.s.layout.state(files).expect("pane").location.clone())
        .expect("refresh");
    assert!(pump(&mut r.s, |s| s.pane_view_ready(files)));
    r.cursor("sub");
    r.wait_static();
    r.s.dispatch("preview.thumbs.toggle", &json!({}))
        .expect("on");
    r.s.dispatch("preview.thumbs.visible", &json!({"first": 0, "count": 5}))
        .expect("visible");
    png("b.png");
    let sub_uri = VfsUri::parse(&sub.to_string_lossy()).expect("uri");
    r.s.refresh_read(sub_uri).expect("refresh");
    let p = r.preview;
    let found = pump(&mut r.s, |s| {
        let Some(Body::Static(a)) = s.preview_slot(p).map(|x| &x.body) else {
            return false;
        };
        let Preview::Dir(d) = &**a else { return false };
        d.items
            .iter()
            .find(|e| e.name == "b.png")
            .is_some_and(|e| !matches!(s.thumbs.get(e, 160), ThumbState::None))
    });
    assert!(found, "new image was never requested");
}
