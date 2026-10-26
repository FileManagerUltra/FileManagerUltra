//! Image snapshots of the pane widgets, dark and light (capability `panes`,
//! task 8.2). Snapshots skip (with a note) when no GPU adapter is available —
//! CI machines without wgpu must not fail the build.
//!
//! Update with `UPDATE_SNAPSHOTS=1 cargo test -p fm-ui --test snapshots`
//! (see `docs/development.md`).
#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use egui_kittest::{Harness, SnapshotResults, wgpu::WgpuTestRenderer};
use fm_engine::Session;
use fm_testkit::MemFs;
use fm_ui::theme::{self, Tokens};
use fm_ui::{listing, pane, tree};
use fm_vfs::{Vfs, VfsUri};

const FIXED_MTIME: SystemTime = SystemTime::UNIX_EPOCH;

/// GPU adapters are a process-wide resource: serialize snapshot tests so
/// parallel test threads don't race adapter creation.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_gpu() -> std::sync::MutexGuard<'static, ()> {
    GPU.lock().unwrap_or_else(|p| p.into_inner())
}

fn wait_ready(s: &mut Session, uri: &VfsUri) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        let state = s.dirs.state(uri);
        if state.is_ready() || matches!(state, fm_engine::DirState::Failed { .. }) {
            return;
        }
        assert!(Instant::now() < deadline, "read never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn fixture_session() -> Session {
    let mem = MemFs::new();
    mem.mkdirs_str("/home/alice/docs").expect("mkdirs");
    mem.mkdirs_str("/home/alice/pics").expect("mkdirs");
    mem.put_file_at(
        &VfsUri::parse("/home/alice/notes.txt").expect("uri"),
        b"hello snapshot",
        FIXED_MTIME,
    )
    .expect("put");
    mem.put_file_at(
        &VfsUri::parse("/home/alice/big.bin").expect("uri"),
        &[0u8; 4096],
        FIXED_MTIME,
    )
    .expect("put");
    let mut s = Session::default();
    s.backends.register(Arc::new(mem));
    let uri = VfsUri::parse("/home/alice").expect("uri");
    s.layout.state_mut(1).expect("pane").location = uri.clone();
    s.submit_read(uri.clone()).expect("submit");
    wait_ready(&mut s, &uri);
    s
}

fn renderer_or_skip() -> Option<WgpuTestRenderer> {
    match std::panic::catch_unwind(WgpuTestRenderer::new) {
        Ok(renderer) => Some(renderer),
        Err(_) => {
            eprintln!("skipping snapshot: no wgpu adapter available");
            None
        }
    }
}

#[test]
fn one_pane_empty_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let mem = MemFs::new();
        let mut s = Session::default();
        s.backends.register(Arc::new(mem));
        let uri = VfsUri::parse("/").expect("uri");
        s.layout.state_mut(1).expect("pane").location = uri.clone();
        s.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut s, &uri);
        let mut session = s;
        let name = format!("one_pane_empty_{theme}");
        match renderer_or_skip() {
            Some(renderer) => {
                let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
                    theme::apply_to(ui.ctx(), &tokens);
                    let _ = pane::show(ui, &mut session, 1, &tokens);
                });
                harness.set_size(egui::vec2(640.0, 400.0));
                harness.run();
                harness.snapshot(&name);
                results.extend_harness(&mut harness);
            }
            None => return,
        }
    }
}

#[test]
fn one_pane_populated_selected_dark() {
    let _gpu = lock_gpu();
    let tokens = Tokens::dark();
    let mut session = fixture_session();
    {
        let state = session.layout.state_mut(1).expect("pane");
        state.listing.select_only(Some("notes.txt"), 2);
    }
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
        theme::apply_to(ui.ctx(), &tokens);
        let _ = pane::show(ui, &mut session, 1, &tokens);
    });
    harness.set_size(egui::vec2(640.0, 400.0));
    harness.run();
    harness.snapshot("one_pane_populated_selected_dark");
}

#[test]
fn tree_expanded_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let mut session = fixture_session();
        {
            let state = session.layout.state_mut(1).expect("pane");
            state
                .tree
                .expand(VfsUri::parse("/home/alice/docs").expect("uri"));
        }
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let name = format!("tree_expanded_{theme}");
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            let _ = tree::show(ui, &mut session, 1, &tokens);
        });
        harness.set_size(egui::vec2(320.0, 240.0));
        harness.run();
        harness.snapshot(&name);
        results.extend_harness(&mut harness);
    }
}

#[test]
fn two_pane_split_dark() {
    let _gpu = lock_gpu();
    let tokens = Tokens::dark();
    let mut session = fixture_session();
    session
        .layout
        .split(1, fm_layout::Axis::Horizontal, 0.5)
        .expect("split");
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
        theme::apply_to(ui.ctx(), &tokens);
        // Mirror the app's split renderer: `ui.horizontal` alone gives the row
        // only `interact_size.y` of height, so each cell must be told to take
        // the full available height or the panes collapse into a strip.
        ui.horizontal_top(|ui| {
            let cell_w = ((ui.available_width() - 6.0) / 2.0).max(0.0);
            for id in [1, 2] {
                ui.allocate_ui_with_layout(
                    egui::vec2(cell_w, ui.available_height()),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        let _ = pane::show(ui, &mut session, id, &tokens);
                    },
                );
            }
        });
    });
    harness.set_size(egui::vec2(800.0, 400.0));
    harness.run();
    harness.snapshot("two_pane_split_dark");
}

#[test]
fn loading_state_dark() {
    let _gpu = lock_gpu();
    let tokens = Tokens::dark();
    let mut session = Session::default();
    // No read submitted: the listing shows the loading affordance.
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
        theme::apply_to(ui.ctx(), &tokens);
        let _ = pane::show(ui, &mut session, 1, &tokens);
    });
    harness.set_size(egui::vec2(640.0, 400.0));
    harness.run();
    harness.snapshot("loading_state_dark");
}

#[test]
fn error_with_retry_dark() {
    let _gpu = lock_gpu();
    let tokens = Tokens::dark();
    let mut session = Session::default();
    // Synthesize a failed read with a stable message (real paths would make
    // the snapshot machine-dependent).
    let uri = VfsUri::parse("/denied-fixture").expect("uri");
    session.layout.state_mut(1).expect("pane").location = uri.clone();
    session.dirs.begin_load(uri.clone(), 1);
    session.dirs.apply(
        &uri,
        fm_engine::ReadOutcome::Err("Permission denied (fixture)".to_string()),
    );
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
        theme::apply_to(ui.ctx(), &tokens);
        let _ = pane::show(ui, &mut session, 1, &tokens);
    });
    harness.set_size(egui::vec2(640.0, 400.0));
    harness.run();
    harness.snapshot("error_with_retry_dark");
}

// Keep listing/tree public API referenced so refactors that change signatures
// fail this module's compile even if snapshots are skipped.
#[test]
fn widgets_are_reachable() {
    let _gpu = lock_gpu();
    let tokens = Tokens::light();
    let mut session = Session::default();
    let ctx = egui::Context::default();
    theme::apply_to(&ctx, &tokens);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = listing::show(ui, &mut session, 1, &tokens, true);
        });
    });
    output.textures_delta.clear();
}

#[test]
fn dialogs_dark_light() {
    use fm_ui::dialogs::{Dialog, Dialogs};
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let dir = VfsUri::parse("/home/alice").expect("uri");
    let cases = [
        (
            "mkdir_invalid",
            Dialog::Mkdir {
                dir: dir.clone(),
                name: "docs".to_string(),
            },
        ),
        (
            "rename",
            Dialog::Rename {
                dir: dir.clone(),
                current: "notes.txt".to_string(),
                name: "notes.md".to_string(),
            },
        ),
        (
            "confirm_delete",
            Dialog::ConfirmDelete {
                sources: vec![dir.join("notes.txt").expect("uri")],
            },
        ),
        (
            "destination",
            Dialog::Destination {
                command: "file.copy",
                sources: vec![dir.join("notes.txt").expect("uri")],
                dest: "/home/alice/docs".to_string(),
            },
        ),
        (
            "confirm_run",
            Dialog::ConfirmRun {
                uri: dir.join("install.sh").expect("uri"),
                name: "install.sh".to_string(),
            },
        ),
    ];
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for (case, dialog) in &cases {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let mut session = fixture_session();
            let mut dialogs = Dialogs::default();
            dialogs.request(dialog.clone());
            let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                dialogs.show(ui.ctx(), &mut session, &tokens);
            });
            harness.set_size(egui::vec2(480.0, 240.0));
            harness.run();
            harness.snapshot(format!("dialog_{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn preferences_dialog_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.preferences_open = true;
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            fm_ui::preferences::show(ui.ctx(), &mut session, &tokens);
        });
        harness.set_size(egui::vec2(560.0, 300.0));
        harness.run();
        harness.snapshot(format!("dialog_preferences_{theme}"));
        results.extend_harness(&mut harness);
    }
}

/// 1×1 RGBA PNG (the same fixture the preview unit tests use).
const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d, 0x49, 0x48, 0x44, 0x52, 0, 0, 0,
    1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1f, 0x15, 0xc4, 0x89, 0, 0, 0, 0x0d, 0x49, 0x44, 0x41, 0x54,
    0x78, 0x9c, 0x63, 0x60, 0x64, 0x62, 0xfe, 0xf, 0, 0, 2, 0, 1, 0xe2, 0x21, 0xbc, 0x33, 0, 0, 0,
    0, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Session with a Preview pane showing `name` (a MemFs file with `bytes`).
///
/// With `vanish`, the file is removed after the directory read: the preview job then
/// fails and the pane renders its error card — a deterministic fixture for that state.
fn preview_fixture(name: &str, bytes: &[u8], vanish: bool) -> (Session, u64) {
    let mem = MemFs::new();
    mem.mkdirs_str("/home/alice").expect("mkdirs");
    mem.put_str(&format!("/home/alice/{name}"), bytes)
        .expect("put");
    let mem = Arc::new(mem);
    let mut s = Session::new().expect("session");
    s.backends.register(mem.clone());
    let dir = VfsUri::parse("/home/alice").expect("uri");
    {
        let st = s.layout.state_mut(1).expect("pane");
        st.location = dir.clone();
        st.listing.on_parent = false;
        st.listing.focused_row = 0;
    }
    s.submit_read(dir.clone()).expect("submit");
    wait_ready(&mut s, &dir);
    s.dispatch("view.preview.toggle", &serde_json::json!({}))
        .expect("toggle");
    let pane = *s.layout.panes().last().expect("panes");
    if vanish {
        mem.remove(&VfsUri::parse(&format!("/home/alice/{name}")).expect("uri"))
            .expect("remove");
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        s.poll();
        let settled = s
            .preview_slot(pane)
            .is_some_and(|slot| matches!(slot.body, fm_engine::preview_cmds::Body::Static(_)));
        if settled {
            break;
        }
        assert!(Instant::now() < deadline, "preview never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    (s, pane)
}

/// Text, image and error renderers in both palettes (capability `preview-pane`).
#[test]
fn preview_cards_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let cases: [(&str, &str, &[u8], bool); 3] = [
        (
            "text",
            "main.rs",
            b"fn main() {\n    println!(\"hello, preview\");\n}\n",
            false,
        ),
        ("image", "pic.png", TINY_PNG, false),
        ("error", "gone.txt", b"gone", true),
    ];
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for (case, name, bytes, vanish) in cases {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let (mut session, pane) = preview_fixture(name, bytes, vanish);
            // egui's ComboBox exposes its value but no accessible *name*, so the
            // generic accessibility check cannot pass for any preview toolbar.
            let mut harness = Harness::builder()
                .renderer(renderer)
                .with_accessibility_check(false)
                .build_ui(move |ui| {
                    theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::preview::show(ui, &mut session, pane, &tokens);
                });
            harness.set_size(egui::vec2(560.0, 400.0));
            harness.run();
            harness.snapshot(format!("preview_{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

/// A file longer than the pane must fill it down to the bottom edge.
#[test]
fn preview_long_text_fills_the_pane() {
    let _gpu = lock_gpu();
    let Some(renderer) = renderer_or_skip() else {
        return;
    };
    let tokens = Tokens::dark();
    let body: String = (1..=200).map(|i| format!("line {i}\n")).collect();
    let (mut session, pane) = preview_fixture("long.txt", body.as_bytes(), false);
    let mut harness = Harness::builder()
        .renderer(renderer)
        .with_accessibility_check(false)
        .build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            fm_ui::preview::show(ui, &mut session, pane, &tokens);
        });
    harness.set_size(egui::vec2(400.0, 300.0));
    harness.run();
    harness.snapshot("preview_text_long_dark");
}

#[test]
fn preview_toolbar_collapses_into_a_menu_when_narrow() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (mut session, pane) = preview_fixture("main.rs", b"fn main() {}\n", false);
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_accessibility_check(false)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                fm_ui::preview::show(ui, &mut session, pane, &tokens);
            });
        harness.set_size(egui::vec2(220.0, 200.0));
        harness.run();
        harness.snapshot(format!("preview_narrow_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn connect_dialog_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.prefs.profiles.push(fm_engine::prefs::Profile {
            name: "nas".into(),
            host: "nas.local".into(),
            port: 22,
            user: "alice".into(),
            key_path: None,
            start_dir: None,
        });
        session.layout.state_mut(1).expect("pane").kind = fm_layout::PaneKind::Sftp;
        session.layout.state_mut(1).expect("pane").location =
            VfsUri::parse("/home/alice").expect("uri");
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            let _ = pane::show(ui, &mut session, 1, &tokens);
        });
        harness.set_size(egui::vec2(640.0, 460.0));
        harness.run();
        harness.snapshot(format!("pane_sftp_connect_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn trash_pane_dark_light() {
    use fm_engine::trash_cmds::{TrashEntry, TrashListing};
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = Session::default();
        session.trash.loaded = true;
        session.trash.listing = TrashListing {
            entries: ["report.pdf", "old-notes.txt", "backup"]
                .iter()
                .enumerate()
                .map(|(i, name)| TrashEntry {
                    id: format!("id{i}"),
                    name: (*name).to_string(),
                    original_parent: "/home/alice/Documents".to_string(),
                    deleted: 1_782_393_780 - i as i64 * 86_400,
                    is_dir: *name == "backup",
                })
                .collect(),
            truncated: false,
        };
        session
            .trash
            .selected_children
            .insert(("id2".to_string(), vec!["notes.txt".to_string()]));
        let child = |name: &str, is_dir: bool| fm_engine::trash_cmds::TrashChild {
            name: name.to_string(),
            is_dir,
            size: 2048,
        };
        session.trash.expanded.insert(
            fm_engine::trash_cmds::child_key("id2", &[]),
            fm_engine::trash_cmds::Children::Ready(fm_engine::trash_cmds::TrashChildren {
                entries: vec![child("photos", true), child("notes.txt", false)],
                truncated: false,
            }),
        );
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_accessibility_check(false)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                fm_ui::trash::show(ui, &mut session, 1, &tokens);
            });
        harness.set_size(egui::vec2(560.0, 300.0));
        harness.run();
        harness.snapshot(format!("pane_trash_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn sftp_pane_badge_and_stale_banner_dark_light() {
    use fm_engine::remote::ConnState;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = Session::default();
        let uri = VfsUri::parse("sftp://alice@nas.local/home/alice").expect("uri");
        session.layout.state_mut(1).expect("pane").location = uri.clone();
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            let states = [
                ConnState::Ready,
                ConnState::Connecting,
                ConnState::Failed("connection reset by peer".to_string()),
            ];
            for state in states {
                session.remote.set_state_for_tests(uri.authority(), state);
                for focused in [true, false] {
                    egui::Frame::NONE
                        .fill(if focused { tokens.accent } else { tokens.panel })
                        .inner_margin(4.0)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label("pane");
                                fm_ui::remote::badge(ui, &mut session, 1, &tokens, focused);
                            });
                        });
                }
                fm_ui::remote::stale_banner(ui, &mut session, 1, &tokens);
                ui.add_space(8.0);
            }
        });
        harness.set_size(egui::vec2(480.0, 360.0));
        harness.run();
        harness.snapshot(format!("pane_sftp_badge_stale_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn changed_host_key_card_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.remote.host_key_alert = Some(
            "the host key of nas.local:22 changed (trusted SHA256:aaaa, presented SHA256:bbbb); \
             refusing to connect — this may be an attack"
                .to_string(),
        );
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            fm_ui::remote::host_key_alert(ui.ctx(), &mut session, &tokens);
        });
        harness.set_size(egui::vec2(560.0, 300.0));
        harness.run();
        harness.snapshot(format!("dialog_hostkey_changed_{theme}"));
        results.extend_harness(&mut harness);
    }
}
