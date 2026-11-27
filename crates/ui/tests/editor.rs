//! Edit mode of the Preview pane (capability `text-editor`): snapshots in dark and light
//! (coloured code, find bar, unsaved-changes warning) and a scroll soak over a large
//! buffer. Snapshots skip when no GPU adapter is available.
//!
//! Update with `UPDATE_SNAPSHOTS=1 cargo test -p fm-ui --test editor`.
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui_kittest::{Harness, SnapshotResults, kittest::Queryable, wgpu::WgpuTestRenderer};
use fm_engine::Session;
use fm_engine::editor_doc::Phase;
use fm_layout::PaneId;
use fm_testkit::MemFs;
use fm_ui::theme::{self, Tokens};
use fm_vfs::VfsUri;
use serde_json::json;

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

const RUST: &str = r#"// Rainbow of tokens.
use std::collections::HashMap;

/// Counts words.
pub fn count(text: &str) -> HashMap<&str, usize> {
    let mut out = HashMap::new();
    for word in text.split_whitespace() {
        *out.entry(word).or_insert(0) += 1;
    }
    out
}

fn main() {
    let n = 42;
    println!("{n} {:?}", count("a b a"));
}
"#;

fn renderer_or_skip() -> Option<WgpuTestRenderer> {
    match std::panic::catch_unwind(WgpuTestRenderer::new) {
        Ok(renderer) => Some(renderer),
        Err(_) => {
            eprintln!("skipping snapshot: no wgpu adapter available");
            None
        }
    }
}

/// A session editing `name` with `text`, loaded (and parsed, for code).
fn editing(name: &str, text: &str) -> (Session, PaneId) {
    let mem = MemFs::new();
    let uri = VfsUri::parse(&format!("/w/{name}")).expect("uri");
    mem.put_file_at(&uri, text.as_bytes(), SystemTime::UNIX_EPOCH)
        .expect("put");
    let mut s = Session::new().expect("session");
    s.backends.register(Arc::new(mem));
    s.dispatch("editor.open", &json!({ "uri": uri.to_string() }))
        .expect("open");
    let pane = s.layout.focused();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        s.poll();
        let ready = s.editor(pane).is_some_and(|d| {
            d.phase == Phase::Ready
                && d.syntax
                    .as_ref()
                    .is_none_or(|syn| syn.is_current(&d.buffer))
        });
        if ready {
            break;
        }
        assert!(Instant::now() < deadline, "editor never loaded");
        std::thread::sleep(Duration::from_millis(5));
    }
    (s, pane)
}

fn snapshot(
    name: &str,
    mut s: Session,
    pane: PaneId,
    tokens: Tokens,
    results: &mut SnapshotResults,
) {
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    // Sized from the first frame: dialogs keep the position they open at.
    let mut harness = Harness::builder()
        .renderer(renderer)
        .with_size(egui::vec2(640.0, 420.0))
        .build_ui(move |ui| {
            ui.ctx().set_theme(if tokens.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            theme::apply_to(ui.ctx(), &tokens);
            fm_ui::editor::show(ui, &mut s, pane, &tokens);
        });
    harness.run();
    harness.snapshot(name);
    results.extend_harness(&mut harness);
}

#[test]
fn coloured_code_dark_light() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let (mut s, pane) = editing("main.rs", RUST);
        s.dispatch("editor.goto_line", &json!({"line": 6, "column": 12}))
            .expect("caret");
        snapshot(
            &format!("editor_code_{theme}"),
            s,
            pane,
            tokens,
            &mut results,
        );
    }
}

#[test]
fn find_bar_and_selection_dark_light() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let (mut s, pane) = editing("main.rs", RUST);
        s.dispatch(
            "editor.find",
            &json!({"query": "count", "replace": true, "replacement": "tally"}),
        )
        .expect("find");
        s.dispatch("editor.find_next", &json!(null)).expect("next");
        snapshot(
            &format!("editor_find_{theme}"),
            s,
            pane,
            tokens,
            &mut results,
        );
    }
}

#[test]
fn unsaved_warning_names_the_autosave_preference_dark_light() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let (mut s, pane) = editing("notes.txt", "Shopping\n- milk\n- bread\n");
        s.prefs.editor.autosave = false;
        s.dispatch("editor.input", &json!({"op": "insert", "text": "Weekend "}))
            .expect("type");
        s.dispatch("editor.toggle", &json!(null)).expect("leave");
        assert!(s.editor(pane).expect("doc").leave_prompt);
        snapshot(
            &format!("editor_unsaved_{theme}"),
            s,
            pane,
            tokens,
            &mut results,
        );
    }
}

/// The editor lays out only what is visible: scrolling a 400 000-line buffer from top
/// to bottom, frame by frame, stays fast and never needs the whole text laid out.
#[test]
fn scrolling_a_large_buffer_stays_fast() {
    let line = "the quick brown fox jumps over the lazy dog 0123456789 abcdefghij\n";
    let text = line.repeat(400_000);
    let (s, pane) = editing("big.txt", &text);
    let tokens = Tokens::dark();
    let session = std::rc::Rc::new(std::cell::RefCell::new(s));
    let shared = session.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_ui(move |ui| {
            fm_ui::editor::show(ui, &mut shared.borrow_mut(), pane, &tokens);
        });
    harness.run_steps(2);
    let started = Instant::now();
    for step in 0..40u64 {
        let line = step * 10_000;
        session
            .borrow_mut()
            .dispatch("editor.goto_line", &json!({ "line": line + 1 }))
            .expect("goto");
        harness.run_steps(1);
    }
    let per_frame = started.elapsed() / 40;
    let s = session.borrow();
    let doc = s.editor(pane).expect("doc");
    assert!(doc.scroll_line > 300_000, "view followed the caret");
    assert!(
        per_frame < Duration::from_millis(500),
        "a frame took {per_frame:?}; the view must not scale with the file"
    );
}

#[test]
fn folded_code_and_rainbow_brackets_dark_light() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let (mut s, pane) = editing("main.rs", RUST);
        s.dispatch("editor.fold", &json!({"line": 5}))
            .expect("fold count()");
        assert!(
            !s.editor(pane).expect("doc").folds.is_empty(),
            "the function folded"
        );
        s.dispatch("editor.goto_line", &json!({"line": 15, "column": 14}))
            .expect("caret in nested brackets");
        snapshot(
            &format!("editor_folded_{theme}"),
            s,
            pane,
            tokens,
            &mut results,
        );
    }
}

#[test]
fn live_preview_split_dark_light() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let (mut s, editor) = editing("notes.md", "# Notes\n\nSaved text.\n");
        s.dispatch("editor.open_live_preview", &json!(null))
            .expect("live preview");
        s.dispatch("editor.input", &json!({"op": "move", "to": "doc_end"}))
            .expect("end");
        s.dispatch(
            "editor.input",
            &json!({"op": "insert", "text": "- an unsaved bullet\n"}),
        )
        .expect("type");
        let follower = s
            .layout
            .panes()
            .into_iter()
            .find(|p| *p != editor && s.pane_kind(*p) == Some(fm_layout::PaneKind::Preview))
            .expect("follower");
        let deadline = Instant::now() + Duration::from_secs(10);
        while !s.previews.slot(follower).is_some_and(|slot| {
            matches!(&slot.body, fm_engine::preview_cmds::Body::Static(p) if format!("{p:?}").contains("unsaved bullet"))
        }) {
            assert!(Instant::now() < deadline, "live preview never rendered");
            s.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            s.editor(editor).expect("doc").dirty(),
            "typing alone saves nothing"
        );
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            ui.ctx().set_theme(if tokens.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            theme::apply_to(ui.ctx(), &tokens);
            ui.columns(2, |cols| {
                if let [left, right] = cols {
                    fm_ui::editor::show(left, &mut s, editor, &tokens);
                    fm_ui::preview::show(right, &mut s, follower, &tokens);
                }
            });
        });
        harness.set_size(egui::vec2(900.0, 360.0));
        harness.run();
        harness.snapshot(format!("editor_live_preview_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn colour_schemes_dark_light() {
    use fm_engine::editor_prefs::EditorScheme;
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    for scheme in [EditorScheme::Warm, EditorScheme::HighContrast] {
        for (mut tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
            tokens.code = theme::CodePalette::of(scheme, tokens.dark);
            if scheme == EditorScheme::HighContrast {
                tokens.rainbow = theme::CodePalette::contrast_rainbow(tokens.dark);
            }
            let (mut s, pane) = editing("main.rs", RUST);
            s.prefs.editor.scheme = scheme;
            s.dispatch("editor.goto_line", &json!({"line": 6, "column": 12}))
                .expect("caret");
            snapshot(
                &format!("editor_scheme_{}_{theme}", scheme.id()),
                s,
                pane,
                tokens,
                &mut results,
            );
        }
    }
}

/// Preferences › Editor › Opening files: one segmented choice per kind of file.
#[test]
fn preferences_choose_the_editor_per_kind() {
    use fm_engine::editor_prefs::{EditorKind, EditorTarget};
    let mut s = Session::new().expect("session");
    s.preferences_open = true;
    let tokens = Tokens::dark();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 1400.0))
        .build_ui_state(
            move |ui, s| {
                theme::apply_to(ui.ctx(), &tokens);
                fm_ui::preferences::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    h.run();
    h.get_by_label("Editor").click();
    h.run();
    h.get_by_label("Logs: External editor").scroll_to_me();
    h.run();
    h.get_by_label("Logs: External editor").click();
    h.run();
    let open = &h.state().prefs.editor.open;
    assert_eq!(open.get(EditorKind::Log), EditorTarget::External);
    assert_eq!(
        open.get(EditorKind::Code),
        EditorTarget::Internal,
        "only that kind"
    );
}

/// The pane's zoom scales the editor's text (not its toolbar), like the rendered preview.
#[test]
fn pane_zoom_scales_the_text() {
    let _gpu = GPU.lock().unwrap_or_else(|p| p.into_inner());
    let mut results = SnapshotResults::new();
    let (mut s, pane) = editing("main.rs", RUST);
    for _ in 0..2 {
        s.dispatch("preview.scale.in", &json!({ "pane": pane }))
            .expect("zoom in");
    }
    let scale = s.preview_slot(pane).map_or(1.0, |slot| slot.scale);
    assert!(scale > 1.2, "zoomed to {scale}");
    snapshot("editor_zoomed_dark", s, pane, Tokens::dark(), &mut results);
}

/// "Save changes?": the title-bar × and Escape cancel (the edits stay), Don't save drops
/// them, and the hint opens Preferences on the Editor page.
#[test]
fn save_prompt_closes_as_cancel_and_links_to_preferences() {
    for how in ["title bar", "escape", "hint", "discard"] {
        let (mut s, pane) = editing("notes.txt", "Shopping\n");
        s.prefs.editor.autosave = false;
        s.dispatch("editor.input", &json!({"op": "insert", "text": "Weekend "}))
            .expect("type");
        s.dispatch("editor.toggle", &json!(null)).expect("leave");
        assert!(s.editor(pane).expect("doc").leave_prompt);
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 500.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::editor::show(ui, s, pane, &tokens);
                },
                s,
            );
        h.run();
        match how {
            "title bar" => h.get_by_label("Close window").click(),
            "escape" => h.key_press(egui::Key::Escape),
            "hint" => h.get_by_label("Turn it on in Preferences").click(),
            _ => h.get_by_label("Don't save").click(),
        }
        h.run();
        let s = h.state();
        let doc = s.editor(pane);
        if how == "discard" {
            assert!(doc.is_none_or(|d| !d.dirty()), "edits dropped");
        } else {
            let doc = doc.expect("still editing");
            assert!(!doc.leave_prompt, "{how}: prompt closed");
            assert!(doc.dirty(), "{how}: edits kept");
        }
        assert_eq!(s.preferences_open, how == "hint", "{how}");
    }
}
