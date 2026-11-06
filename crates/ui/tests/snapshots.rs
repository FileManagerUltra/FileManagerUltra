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
    let mut s = test_session();
    s.backends.register(Arc::new(mem));
    let uri = VfsUri::parse("/home/alice").expect("uri");
    s.layout.state_mut(1).expect("pane").location = uri.clone();
    s.submit_read(uri.clone()).expect("submit");
    wait_ready(&mut s, &uri);
    s
}

/// Places come from the machine's user folders; snapshots need the same list everywhere.
fn with_fixed_places(mut s: Session) -> Session {
    s.standard_places =
        fm_engine::places_cmds::StandardPlaces(vec![fm_engine::places_cmds::Place {
            name: "Home".to_string(),
            uri: VfsUri::parse("/home/alice").expect("uri"),
            bookmark: false,
            kind: fm_engine::places_cmds::PlaceKind::Home,
        }]);
    s
}

fn test_session() -> Session {
    with_fixed_places(Session::default())
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
        let mut s = test_session();
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
    let mut session = test_session();
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
    let mut session = test_session();
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
    let mut session = test_session();
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
            "select_pattern",
            Dialog::SelectPattern {
                pattern: "*.txt".to_string(),
                mode: "add",
            },
        ),
        (
            "rename_bookmark",
            Dialog::RenameBookmark {
                uri: "/home/alice/docs".to_string(),
                name: "Docs".to_string(),
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
            let mut harness = Harness::builder()
                .with_size(egui::vec2(480.0, 260.0))
                .renderer(renderer)
                .build_ui(move |ui| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    dialogs.show(ui.ctx(), &mut session, &tokens);
                });
            harness.run();
            harness.run_steps(3);
            harness.snapshot(format!("dialog_{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn preferences_dialog_dark_light() {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.preferences_open = true;
        session.registry =
            fm_engine::Registry::from_specs(fm_engine::commands::specs()).expect("registry");
        let hosts_dir = tempfile::tempdir().expect("known-host directory");
        std::fs::write(
            hosts_dir.path().join(fm_engine::remote::KNOWN_HOSTS_FILE),
            json!({
                "hosts": [
                    {
                        "host": "192.168.0.50:22",
                        "algorithm": "ssh-ed25519",
                        "fingerprint": "SHA256:fixture",
                        "openssh": "AAAAC3NzaC1lZDI1NTE5AAAAfixture"
                    },
                    {
                        "host": "files.example.test:22",
                        "algorithm": "ssh-ed25519",
                        "fingerprint": "SHA256:fixture",
                        "openssh": "AAAAC3NzaC1lZDI1NTE5AAAAfixture"
                    }
                ]
            })
            .to_string(),
        )
        .expect("known hosts");
        session.remote.set_config_dir(hosts_dir.path().into());
        for name in ["Files", "Remote work", "Logs"] {
            session
                .dispatch("workspace.new", &json!({"name": name}))
                .expect("new");
            session
                .dispatch("workspace.save", &json!({}))
                .expect("save");
        }
        // A fixed location keeps the Advanced page independent of the machine's home.
        session.prefs.thumb_cache_dir = Some("/home/user/.cache/fm".to_string());
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 650.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::preferences::show(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("dialog_preferences_{theme}"));
        for topic in [
            "Appearance",
            "Files",
            "Copy and move",
            "Preview",
            "Terminal",
            "Connections",
            "Workspaces",
            "Keyboard shortcuts",
            "Advanced",
        ] {
            harness.get_by_label(topic).click();
            harness.run();
            harness.run_steps(3);
            harness.snapshot(format!(
                "dialog_preferences_{}_{theme}",
                topic.to_lowercase().replace(' ', "_")
            ));
            if topic == "Keyboard shortcuts" {
                harness.get_by_label("Search shortcuts").click();
                harness
                    .get_by_label("Search shortcuts")
                    .type_text("terminal pane");
                harness.run_steps(3);
                harness.get_by_label("Terminal pane");
                assert!(
                    harness
                        .query_by_label("Application and appearance")
                        .is_none()
                );
                harness.snapshot(format!("dialog_preferences_shortcuts_filtered_{theme}"));
                harness.set_size(egui::vec2(560.0, 320.0));
                harness.run_steps(3);
                harness.snapshot(format!("dialog_preferences_shortcuts_narrow_{theme}"));
                harness.set_size(egui::vec2(900.0, 650.0));
                harness.run_steps(3);
            }
            if topic == "Preview" {
                harness.get_by_label("Logs").scroll_to_me();
                harness.run();
                harness.snapshot(format!("dialog_preferences_file_filters_{theme}"));
                harness.get_by_label("Images").click();
                harness.run_steps(3);
                harness.get_by_label("Add Images pattern").scroll_to_me();
                harness.run_steps(3);
                harness.snapshot(format!("dialog_preferences_file_filters_open_{theme}"));
                harness
                    .get_by_label("Restore default file filters")
                    .scroll_to_me();
                harness.run();
                harness.snapshot(format!("dialog_preferences_file_filters_bottom_{theme}"));
            }
        }
        results.extend_harness(&mut harness);
    }
}

#[test]
fn update_notification_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for (case, status) in [
            (
                "available",
                fm_engine::updates::UpdateStatus::Available {
                    version: "0.2.0".into(),
                    url: "https://github.com/releases".into(),
                },
            ),
            (
                "failed",
                fm_engine::updates::UpdateStatus::Failed(
                    "Cannot check for updates: network unavailable".into(),
                ),
            ),
        ] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let mut session = fixture_session();
            session.updates.status = status;
            session.updates.visible = true;
            session.registry =
                fm_engine::Registry::from_specs(fm_engine::commands::specs()).expect("registry");
            let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label("The file manager remains usable.");
                });
                fm_ui::jobs::show(ui.ctx(), &mut session, &tokens);
            });
            harness.set_size(egui::vec2(820.0, 360.0));
            harness.run();
            harness.snapshot(format!("update_{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
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
    preview_fixture_with(name, bytes, vanish, &[])
}

fn preview_fixture_with(
    name: &str,
    bytes: &[u8],
    vanish: bool,
    extra: &[(&str, &[u8])],
) -> (Session, u64) {
    let mem = MemFs::new();
    mem.mkdirs_str("/home/alice").expect("mkdirs");
    for (n, b) in extra {
        mem.put_str(&format!("/home/alice/{n}"), b).expect("put");
    }
    mem.put_str(&format!("/home/alice/{name}"), bytes)
        .expect("put");
    let mem = Arc::new(mem);
    let mut s = with_fixed_places(Session::new().expect("session"));
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
        let settled = s.preview_slot(pane).is_some_and(|slot| {
            matches!(
                slot.body,
                fm_engine::preview_cmds::Body::Static(_) | fm_engine::preview_cmds::Body::Log(_)
            )
        });
        if settled {
            break;
        }
        assert!(Instant::now() < deadline, "preview never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    (s, pane)
}

#[test]
fn text_and_markdown_select_all_copy_includes_offscreen_rows() {
    for (name, source, expected) in [
        (
            "all.txt",
            (0..100)
                .map(|n| format!("row-{n:03}"))
                .collect::<Vec<_>>()
                .join("\n"),
            (0..100)
                .map(|n| format!("row-{n:03}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        (
            "all.md",
            "# Heading\n\nHello **world**.\n\nLast paragraph.".into(),
            "Heading\nHello world.\nLast paragraph.".into(),
        ),
    ] {
        let (mut s, pane) = preview_fixture(name, source.as_bytes(), false);
        s.layout.focus_pane(pane).expect("focus preview");
        let ctx = egui::Context::default();
        let tokens = Tokens::light();
        let frame = |s: &mut Session, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 180.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
            );
            out.textures_delta.clear();
        };
        frame(&mut s, vec![]);
        frame(
            &mut s,
            vec![egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert!(fm_engine::preview_text_cmds::selected(&s, pane));
        s.preferences_open = true;
        frame(&mut s, vec![egui::Event::Copy]);
        assert!(s.clipboard_out.is_none(), "Preferences owns copy");
        s.preferences_open = false;
        frame(&mut s, vec![egui::Event::Copy]);
        assert_eq!(s.clipboard_out.take().expect("copied text"), expected);
        frame(
            &mut s,
            vec![egui::Event::Key {
                key: egui::Key::C,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        assert_eq!(s.clipboard_out.take().expect("copied text"), expected);
    }
}

#[test]
fn preview_find_navigates_offscreen_text_and_keeps_keyboard_in_the_query() {
    use egui_kittest::kittest::Queryable;
    for (tokens, wrap) in [(Tokens::dark(), false), (Tokens::light(), true)] {
        let body = (0..180)
            .map(|row| {
                if row == 155 {
                    format!("row {row}: {}needle", "long-line ".repeat(30))
                } else if row == 5 {
                    format!("row {row}: needle")
                } else {
                    format!("row {row}: ordinary text")
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let (mut s, pane) = preview_fixture("find.txt", body.as_bytes(), false);
        s.layout.focus_pane(pane).expect("focus preview");
        s.previews.slots.get_mut(&pane).expect("slot").wrap = wrap;
        let mut harness = Harness::builder()
            .with_size(egui::vec2(850.0, 360.0))
            .build_ui_state(
                move |ui, (s, offset, horizontal): &mut (Session, f32, f32)| {
                    theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::preview::show(ui, s, pane, &tokens);
                    let id = ui.make_persistent_id(egui::IdSalt::new(("pv-text", pane)));
                    let scroll = egui::scroll_area::State::load(ui.ctx(), id)
                        .expect("scroll")
                        .clamped_offset();
                    *offset = scroll.y;
                    *horizontal = scroll.x;
                },
                (s, 0.0, 0.0),
            );
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
        harness.run();
        harness.get_by_label("Find in preview").type_text("needle");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            harness.run_steps(2);
            if harness
                .state()
                .0
                .preview_slot(pane)
                .expect("slot")
                .find
                .matches
                .is_some()
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        harness.run_steps(3);
        assert_eq!(
            harness
                .state()
                .0
                .preview_slot(pane)
                .expect("slot")
                .find
                .matches
                .as_ref()
                .expect("matches")
                .hits
                .len(),
            2
        );
        let first = harness.state().1;
        harness.key_press(egui::Key::Enter);
        harness.run_steps(4);
        assert_eq!(
            harness
                .state()
                .0
                .preview_slot(pane)
                .expect("slot")
                .find
                .current,
            1
        );
        assert!(
            harness.state().1 > first + 1000.0,
            "jump to offscreen match"
        );
        if !wrap {
            assert!(
                harness.state().2 > 1000.0,
                "reveal a match beyond the right edge"
            );
        }
        assert_eq!(
            harness.ctx.memory(|m| m.focused()),
            Some(egui::Id::NULL.with(("pv-find-query", pane)))
        );
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::F3);
        harness.run_steps(4);
        assert_eq!(
            harness
                .state()
                .0
                .preview_slot(pane)
                .expect("slot")
                .find
                .current,
            0
        );
        assert!(harness.state().1 < first + 50.0, "jump back");
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(
            !harness
                .state()
                .0
                .preview_slot(pane)
                .expect("slot")
                .find
                .open
        );
        assert!(
            harness
                .state()
                .0
                .layout
                .state(1)
                .expect("files")
                .listing
                .filter
                .is_none()
        );
    }
}

#[test]
fn preview_find_logs_focuses_existing_filters_and_preserves_them_on_escape() {
    use egui_kittest::kittest::Queryable;
    let (mut s, pane) = preview_fixture(
        "find.log",
        b"2026-10-30T12:00:00Z INFO ready\n2026-10-30T12:00:01Z ERROR broken\n",
        false,
    );
    s.layout.focus_pane(pane).expect("focus preview");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 450.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                fm_ui::preview::show(ui, s, pane, &Tokens::dark());
            },
            s,
        );
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
    harness.run();
    assert_eq!(
        harness.ctx.memory(|m| m.focused()),
        Some(egui::Id::NULL.with(("pv-log-query", pane)))
    );
    harness
        .get_by_label("Search log records")
        .type_text("broken");
    harness.run_steps(4);
    assert_eq!(
        harness
            .state()
            .preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .filter
            .query,
        "broken"
    );
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert!(harness.query_by_label("Search log records").is_none());
    assert_eq!(
        harness
            .state()
            .preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .filter
            .query,
        "broken"
    );
}

#[test]
fn preview_find_markdown_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, palette) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (mut s, pane) = preview_fixture("find.md", b"# Finding **needles**\n\nA needle spans **needle** formatting.\n\n> A quoted needle.\n\n```rust\nlet needle = \"needle\";\n```\n\n| Item | Result |\n| --- | --- |\n| needle | found |\n", false);
        s.layout.focus_pane(pane).expect("focus preview");
        s.dispatch("preview.find.open", &serde_json::Value::Null)
            .expect("open");
        s.dispatch(
            "preview.find.set",
            &serde_json::json!({"query": "needle", "whole_word": true}),
        )
        .expect("query");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            fm_engine::preview_find_cmds::prepare(&mut s, pane);
            if s.preview_slot(pane).expect("slot").find.matches.is_some() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            s.preview_slot(pane)
                .expect("slot")
                .find
                .matches
                .as_ref()
                .expect("matches")
                .hits
                .len(),
            6
        );
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 700.0))
            .renderer(renderer)
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        harness.run_steps(4);
        harness.snapshot(format!("preview_find_markdown_{palette}"));
        for expected in [1, 2, 3, 4, 5, 0] {
            harness.key_press(egui::Key::F3);
            harness.run_steps(3);
            let find = &harness.state().preview_slot(pane).expect("slot").find;
            assert_eq!(find.current, expected);
            let reveal = harness
                .ctx
                .data(|d| d.get_temp::<u64>(egui::Id::NULL.with(("pv-find-reveal", pane))));
            assert_eq!(
                reveal,
                Some(find.reveal),
                "active Markdown match was rendered and revealed"
            );
        }
        results.extend_harness(&mut harness);
    }
}

#[test]
fn preview_find_options_recompute_and_report_invalid_regex() {
    use egui_kittest::kittest::Queryable;
    let (mut s, pane) = preview_fixture("find-options.txt", b"Needle needle needles", false);
    s.layout.focus_pane(pane).expect("focus preview");
    s.dispatch("preview.find.open", &serde_json::Value::Null)
        .expect("open");
    let mut harness = Harness::builder()
        .with_size(egui::vec2(850.0, 400.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                fm_ui::preview::show(ui, s, pane, &Tokens::dark());
            },
            s,
        );
    harness.get_by_label("Find in preview").type_text("needle");
    for (toggle, expected) in [(None, 3), (Some("Aa"), 2), (Some("Word"), 1)] {
        if let Some(toggle) = toggle {
            harness.get_by_label(toggle).click();
        }
        harness.run_steps(2);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            harness.run_steps(2);
            let find = &harness.state().preview_slot(pane).expect("slot").find;
            if !find.busy
                && let Some(matches) = &find.matches
            {
                assert_eq!(matches.hits.len(), expected);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    harness.get_by_label(".*").click();
    harness.run_steps(2);
    harness.get_by_label("Find in preview").click();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.get_by_label("Find in preview").type_text("[");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        harness.run_steps(2);
        if harness
            .state()
            .preview_slot(pane)
            .expect("slot")
            .find
            .error
            .is_some()
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        harness.state().preview_text_selection.is_none(),
        "query Ctrl+A must not select the document"
    );
}

#[test]
fn markdown_document_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for (case, width, scale, selected) in [
            ("document", 850.0, 1.0, false),
            ("narrow", 400.0, 1.0, false),
            ("zoomed", 850.0, 1.44, false),
            ("selected", 850.0, 1.0, true),
            ("source", 850.0, 1.0, false),
        ] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let (mut s, pane) =
                preview_fixture("document.md", include_bytes!("fixtures/markdown.md"), false);
            s.layout.focus_pane(pane).expect("focus");
            s.previews.slots.get_mut(&pane).expect("slot").scale = scale;
            if selected {
                s.dispatch("preview.text.select_all", &serde_json::Value::Null)
                    .expect("select");
            }
            let mut harness = Harness::builder()
                .with_size(egui::vec2(width, 1150.0))
                .renderer(renderer)
                .build_ui_state(
                    move |ui, s: &mut Session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        fm_ui::preview::show(ui, s, pane, &tokens);
                    },
                    s,
                );
            harness.run_steps(4);
            if case == "source" {
                use egui_kittest::kittest::Queryable;
                harness.get_by_label("Preview").click();
                harness.run_steps(4);
                assert!(
                    harness
                        .state()
                        .preview_slot(pane)
                        .is_some_and(|slot| slot.source)
                );
            }
            harness.snapshot(format!("markdown_{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn markdown_code_copy_button_and_document_copy_exclude_chrome() {
    use egui_kittest::kittest::Queryable;
    for tokens in [Tokens::dark(), Tokens::light()] {
        for scale in [0.5, 1.0, 2.0] {
            let source = b"# Example\n\n```rust\nlet code = \"copy me\";\n```\n\nAfter.";
            let (mut s, pane) = preview_fixture("copy.md", source, false);
            s.layout.focus_pane(pane).expect("focus preview");
            s.previews.slots.get_mut(&pane).expect("slot").scale = scale;
            let mut harness = Harness::builder()
                .with_size(egui::vec2(850.0, 700.0))
                .build_ui_state(
                    move |ui, s: &mut Session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        fm_ui::preview::show(ui, s, pane, &tokens);
                    },
                    s,
                );
            harness.run();
            let start = harness.get_by_label("Example").rect().left_center() + egui::vec2(0.5, 0.0);
            let end = harness.get_by_label("After.").rect().right_center() - egui::vec2(0.5, 0.0);
            harness.hover_at(start);
            harness.run();
            harness.drag_at(start);
            harness.run();
            harness.hover_at(end);
            harness.run();
            harness.drop_at(end);
            harness.run();
            harness.input_mut().events.push(egui::Event::Copy);
            harness.step();
            let copied = harness
                .output()
                .platform_output
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                    _ => None,
                })
                .expect("drag-selection copy");
            assert!(copied.contains("let code") && copied.contains("After."));
            assert!(!copied.contains("Copy") && !copied.contains("rust"));
            harness.get_by_label("Copy code block").click();
            harness.run();
            assert_eq!(
                harness.state_mut().clipboard_out.take().as_deref(),
                Some("let code = \"copy me\";")
            );
            harness.input_mut().events.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            });
            harness.run();
            harness.input_mut().events.push(egui::Event::Copy);
            harness.run();
            assert_eq!(
                harness.state_mut().clipboard_out.take().as_deref(),
                Some("Example\nlet code = \"copy me\";\nAfter.")
            );
        }
    }
}

#[test]
fn markdown_code_buttons_in_nested_blocks_and_multi_file_cards() {
    use egui_kittest::kittest::Queryable;
    for grid in [false, true] {
        let tokens = Tokens::dark();
        let source = b"```\nfirst\n```\n\n> ```\n> second\n> ```";
        let (mut s, pane) = preview_fixture("nested.md", source, false);
        s.layout.focus_pane(pane).expect("focus preview");
        if grid {
            let slot = s.previews.slots.get_mut(&pane).expect("slot");
            let doc = Arc::new(fm_preview::Preview::Markdown(fm_preview::markdown::MdDoc {
                blocks: fm_preview::markdown::parse("```\ngrid code\n```"),
                source: Default::default(),
                truncated: false,
                total_bytes: 0,
            }));
            let entry = fm_vfs::Entry::new(
                VfsUri::parse("/grid.md").expect("uri"),
                fm_vfs::EntryKind::File,
            );
            let fm_engine::preview_cmds::Body::Static(first) = &slot.body else {
                panic!("static")
            };
            slot.body = fm_engine::preview_cmds::Body::Grid(vec![
                (
                    fm_vfs::Entry::new(
                        VfsUri::parse("/first.md").expect("uri"),
                        fm_vfs::EntryKind::File,
                    ),
                    first.clone(),
                ),
                (entry, doc),
            ]);
        }
        let mut harness = Harness::builder()
            .with_size(egui::vec2(850.0, 700.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        harness.run();
        let target = if grid { 2 } else { 1 };
        harness
            .get_all_by_label("Copy code block")
            .nth(target)
            .expect("target button")
            .click();
        harness.run();
        assert_eq!(
            harness.state_mut().clipboard_out.take().as_deref(),
            Some(if grid { "grid code" } else { "second" })
        );
    }
}

#[test]
fn markdown_code_copy_dark_light() {
    use egui_kittest::kittest::Queryable;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let source = b"```rust\nfn main() {\n    println!(\"Copy just this code\");\n}\n```";
        let (s, pane) = preview_fixture("copy.md", source, false);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(550.0, 300.0))
            .renderer(renderer)
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        harness.run();
        harness.snapshot(format!("markdown_code_copy_idle_{theme}"));
        let pos = harness.get_by_label("Copy code block").rect().center();
        harness
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pos));
        harness.run();
        harness.snapshot(format!("markdown_code_copy_hover_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn markdown_tables_have_aligned_rows_and_natural_text() {
    use egui_kittest::kittest::Queryable;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for (case, width, scale) in [
            ("", 720.0, 1.0),
            ("_narrow", 360.0, 1.0),
            ("_zoomed", 720.0, 1.44),
        ] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let (mut s, pane) = preview_fixture(
                "table.md",
                include_bytes!("fixtures/markdown-table.md"),
                false,
            );
            s.previews.slots.get_mut(&pane).expect("slot").scale = scale;
            let mut harness = Harness::builder()
                .with_size(egui::vec2(width, 900.0))
                .renderer(renderer)
                .build_ui_state(
                    move |ui, s: &mut Session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        fm_ui::preview::show(ui, s, pane, &tokens);
                    },
                    s,
                );
            harness.run_steps(4);
            let mut previous_bottom = None;
            for (left, right) in [
                ("Symptom", "Cause / fix"),
                (
                    "Dashboard still says \"isn't eligible\"",
                    "Wrong source selected in the dashboard header, or the dashboard's time range predates the first scrape. Widen to 24h.",
                ),
                (
                    "node_* absent from Extract metrics",
                    "Vector isn't scraping: ss -tnp '( dport = :9100 )', then journalctl -u vector and grep -i prometheus.",
                ),
                (
                    "connection refused in Vector logs",
                    "node_exporter not running: systemctl status prometheus-node-exporter.",
                ),
            ] {
                let a = harness.get_by_label(left).rect();
                let b = harness.get_by_label(right).rect();
                assert!(
                    (a.top() - b.top()).abs() < 1.0,
                    "{left}: table cells must share a top edge"
                );
                assert!(a.right() < b.left(), "columns must not overlap");
                if let Some(bottom) = previous_bottom {
                    assert!(
                        a.top() > bottom,
                        "next row must clear the tallest previous cell"
                    );
                }
                previous_bottom = Some(a.bottom().max(b.bottom()));
            }
            harness.snapshot(format!("markdown_table{case}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn preview_zoom_shortcuts_scale_each_renderer_without_changing_ui() {
    fn largest_font(shape: &egui::Shape) -> f32 {
        match shape {
            egui::Shape::Text(text) => text
                .galley
                .job
                .sections
                .iter()
                .map(|s| s.format.font_id.size)
                .fold(0.0, f32::max),
            egui::Shape::Vec(shapes) => shapes.iter().map(largest_font).fold(0.0, f32::max),
            _ => 0.0,
        }
    }
    fn largest_image(shape: &egui::Shape) -> f32 {
        match shape {
            egui::Shape::Rect(rect) if rect.brush.is_some() => rect.rect.width(),
            egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::Managed(0) => {
                mesh.calc_bounds().width()
            }
            egui::Shape::Vec(shapes) => shapes.iter().map(largest_image).fold(0.0, f32::max),
            _ => 0.0,
        }
    }
    for (name, bytes) in [
        ("all.txt", b"first\nsecond".as_slice()),
        ("all.md", b"# Heading\n\nParagraph".as_slice()),
        ("all.json", b"{\"a\":1}".as_slice()),
        ("all.bin", b"\0\x01\x02\x03".as_slice()),
        ("all.log", b"2026-01-01 INFO first\n".as_slice()),
        ("all.meta", b"metadata fixture".as_slice()),
        ("all.video", b"video fixture".as_slice()),
        (
            "all.png",
            include_bytes!("../../brand/assets/icon-256.png").as_slice(),
        ),
    ] {
        let (mut s, pane) = preview_fixture(name, bytes, false);
        s.layout.focus_pane(pane).expect("focus");
        if matches!(name, "all.meta" | "all.video") {
            let video = (name == "all.video").then(|| fm_preview::VideoInfo {
                path: "/nonexistent-preview-test.mp4".into(),
                width: 640,
                height: 360,
                fps: 24.0,
                duration: 42.0,
            });
            s.previews.slots.get_mut(&pane).expect("slot").body =
                fm_engine::preview_cmds::Body::Static(Arc::new(fm_preview::Preview::Meta(
                    fm_preview::MetaDoc {
                        rows: vec![("Name".into(), name.into())],
                        hint: None,
                        poster: None,
                        video,
                    },
                )));
            // Initialise state before the UI supplies a decode width; no decoder is spawned.
            s.poll();
            if name == "all.video" {
                let video = s
                    .previews
                    .slots
                    .get_mut(&pane)
                    .expect("slot")
                    .video
                    .as_mut()
                    .expect("video state");
                video.playing = false;
                video.frame = Some(Arc::new(fm_preview::player::Frame {
                    width: 32,
                    height: 18,
                    rgba: vec![128; 32 * 18 * 4],
                    seq: 1,
                }));
            }
        }
        let ctx = egui::Context::default();
        let global_zoom = s.prefs.zoom;
        let frame = |s: &mut Session, key: Option<egui::Key>| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(650.0, 400.0),
                    )),
                    events: key
                        .map(|key| egui::Event::Key {
                            key,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        })
                        .into_iter()
                        .collect(),
                    ..Default::default()
                },
                |ui| {
                    let before = ui.style().text_styles.clone();
                    fm_ui::preview::show(ui, s, pane, &Tokens::light());
                    assert_eq!(
                        before,
                        ui.style().text_styles,
                        "zoom must not leak into other widgets"
                    );
                },
            );
            let font = out
                .shapes
                .iter()
                .map(|shape| largest_font(&shape.shape))
                .fold(0.0, f32::max);
            let image = out
                .shapes
                .iter()
                .map(|shape| largest_image(&shape.shape))
                .fold(0.0, f32::max);
            out.textures_delta.clear();
            (font, image)
        };
        frame(&mut s, None);
        let before = frame(&mut s, None);
        let after = frame(&mut s, Some(egui::Key::Equals));
        if name != "all.video" {
            assert!(
                after.0 > before.0 * 1.1,
                "{name}: rendered body fonts must grow ({before:?} -> {after:?})"
            );
        }
        if matches!(name, "all.png" | "all.video") {
            assert!(
                after.1 > before.1 * 1.1,
                "{name}: actual image dimensions must grow ({before:?} -> {after:?})"
            );
        }
        assert_eq!(s.preview_slot(pane).expect("slot").scale, 1.2, "{name}");
        assert_eq!(s.prefs.zoom, global_zoom);
        assert_eq!(ctx.zoom_factor(), 1.0);
        s.preferences_open = true;
        frame(&mut s, Some(egui::Key::Equals));
        assert_eq!(
            s.preview_slot(pane).expect("slot").scale,
            1.2,
            "Preferences owns keyboard"
        );
        s.preferences_open = false;
        frame(&mut s, Some(egui::Key::Minus));
        assert!((s.preview_slot(pane).expect("slot").scale - 1.0).abs() < 0.001);
        frame(&mut s, Some(egui::Key::Equals));
        frame(&mut s, Some(egui::Key::Num0));
        assert_eq!(s.preview_slot(pane).expect("slot").scale, 1.0);
        assert!(s.notice.is_none(), "{name}: {:?}", s.notice);
    }
}

fn multi_preview_fixture(files: &[(&str, &[u8])]) -> (Session, u64) {
    let fs = MemFs::new();
    fs.mkdirs_str("/logs").expect("mkdir");
    for (name, data) in files {
        fs.put_str(&format!("/logs/{name}"), data).expect("put");
    }
    let mut session = with_fixed_places(Session::new().expect("session"));
    session.backends.register(Arc::new(fs));
    let uri = VfsUri::parse("/logs").expect("uri");
    let state = session.layout.state_mut(1).expect("pane");
    state.location = uri.clone();
    state.listing.on_current = false;
    state.listing.on_parent = false;
    for (index, (name, _)) in files.iter().enumerate() {
        state.listing.toggle(name, index);
    }
    session.submit_read(uri.clone()).expect("read");
    wait_ready(&mut session, &uri);
    session
        .dispatch("view.preview.toggle", &serde_json::json!({}))
        .expect("preview");
    let pane = *session.layout.panes().last().expect("pane");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.poll();
        if session.preview_slot(pane).is_some_and(|s| {
            matches!(
                s.body,
                fm_engine::preview_cmds::Body::Log(_) | fm_engine::preview_cmds::Body::Grid(_)
            )
        }) {
            return (session, pane);
        }
        assert!(Instant::now() < deadline, "combined preview never settled");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn selected_logs_dark_light_and_copy_excludes_filename_gutter() {
    use egui_kittest::kittest::Queryable;

    let _gpu = lock_gpu();
    let first = "2024-05-01 11:00:00 ERROR first";
    let last = "2024-05-01 10:00:00 ERROR last";
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (session, pane) = multi_preview_fixture(&[
            ("a.log", format!("{first}\n#0 stack\n").as_bytes()),
            ("b.log", format!("{last}\n").as_bytes()),
        ]);
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_accessibility_check(false)
            .with_size(egui::vec2(1050.0, 420.0))
            .build_ui_state(
                move |ui, session: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    session.poll();
                    fm_ui::preview::show(ui, session, pane, &tokens);
                },
                session,
            );
        harness.get_by_label("Expand").click();
        harness.run();
        harness.snapshot(format!("selected_logs_concatenated_{theme}"));
        let start = harness.get_by_label(first).rect().left_center() + egui::vec2(0.5, 0.0);
        let end = harness.get_by_label(last).rect().right_center() - egui::vec2(0.5, 0.0);
        harness.hover_at(start);
        harness.run();
        harness.drag_at(start);
        harness.run();
        harness.hover_at(end);
        harness.run();
        harness.drop_at(end);
        harness.run();
        harness.input_mut().events.push(egui::Event::Copy);
        harness.step();
        let copied = harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .expect("copied text");
        assert_eq!(copied, format!("{first}\n#0 stack\n{last}"));
        assert!(!copied.contains("a.log") && !copied.contains("b.log"));
        harness
            .state_mut()
            .dispatch(
                "preview.log.order",
                &serde_json::json!({
                    "mode": "chronological", "pane": pane
                }),
            )
            .expect("order");
        let deadline = Instant::now() + Duration::from_secs(5);
        while harness
            .state()
            .preview_slot(pane)
            .and_then(|s| s.log())
            .is_none_or(|v| v.filtered.is_none())
        {
            harness.run();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        harness.run();
        harness.snapshot(format!("selected_logs_chronological_{theme}"));
        assert!(harness.get_by_label(last).rect().top() < harness.get_by_label(first).rect().top());
        results.extend_harness(&mut harness);
    }
}

#[test]
fn selected_images_and_mixed_cards_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for mixed in [false, true] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let icon = include_bytes!("../../brand/assets/icon-256.png").as_slice();
            let (mut session, pane) = multi_preview_fixture(&[
                ("a.png", icon),
                (
                    if mixed { "b.txt" } else { "b.png" },
                    if mixed {
                        b"Some text\nAnother line\n"
                    } else {
                        icon
                    },
                ),
            ]);
            assert!(matches!(
                &session.preview_slot(pane).expect("slot").body,
                fm_engine::preview_cmds::Body::Grid(items) if matches!(&*items[0].1, fm_preview::Preview::Image(_))
            ));
            let mut harness = Harness::builder()
                .renderer(renderer)
                .with_accessibility_check(false)
                .with_size(egui::vec2(750.0, 450.0))
                .build_ui(move |ui| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    fm_ui::preview::show(ui, &mut session, pane, &tokens);
                });
            harness.snapshot(format!(
                "selected_{}_{theme}",
                if mixed { "mixed" } else { "images" }
            ));
            results.extend_harness(&mut harness);
        }
    }
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
fn preview_page_keys_scroll_only_the_focused_body() {
    use egui_kittest::{Harness, kittest::Queryable};

    for (name, body, salt, at_tail) in [
        (
            "pages.txt",
            (0..400)
                .map(|i| format!("line {i} {}\n", "wide ".repeat(80)))
                .collect::<String>(),
            "pv-text",
            false,
        ),
        (
            "pages.log",
            (0..400)
                .map(|i| format!("2024-05-01 10:00:00 INFO line {i} {}\n", "wide ".repeat(80)))
                .collect::<String>(),
            "pv-log",
            true,
        ),
    ] {
        let (mut session, pane) = preview_fixture(name, body.as_bytes(), false);
        session.layout.focus_pane(pane).expect("focus preview");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 360.0))
            .with_max_steps(16)
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (session, offset, horizontal): &mut (Session, f32, f32)| {
                    fm_ui::preview::show(ui, session, pane, &Tokens::dark());
                    let id = ui.make_persistent_id(egui::IdSalt::new((salt, pane)));
                    let state = egui::scroll_area::State::load(ui.ctx(), id)
                        .expect("scroll state")
                        .clamped_offset();
                    *offset = state.y;
                    *horizontal = state.x;
                },
                (session, 0.0, 0.0),
            );
        let initial = harness.state().1;
        harness.key_press(if at_tail {
            egui::Key::PageUp
        } else {
            egui::Key::PageDown
        });
        harness.run();
        let paged = harness.state().1;
        assert!(
            (paged - initial).abs() > 200.0,
            "{name}: must move by a page"
        );
        assert!((paged - initial).abs() < 360.0, "{name}: keep some overlap");
        harness.key_press(if at_tail {
            egui::Key::PageDown
        } else {
            egui::Key::PageUp
        });
        harness.run();
        assert!(
            (harness.state().1 - initial).abs() < 1.0,
            "{name}: reverse paging"
        );
        let horizontal = harness.state().2;
        harness.key_press(egui::Key::ArrowRight);
        harness.run();
        assert!(
            harness.state().2 > horizontal + 5.0,
            "{name}: Right scrolls horizontally"
        );
        assert!(
            (harness.state().1 - initial).abs() < 1.0,
            "{name}: Right preserves vertical offset"
        );
        harness.key_press(egui::Key::ArrowLeft);
        harness.run();
        assert!(
            (harness.state().2 - horizontal).abs() < 1.0,
            "{name}: Left restores horizontal offset"
        );
        harness.key_press(if at_tail {
            egui::Key::ArrowUp
        } else {
            egui::Key::ArrowDown
        });
        harness.run();
        let moved = (harness.state().1 - initial).abs();
        assert!(
            moved > 5.0 && moved < 40.0,
            "{name}: arrow scrolls one line"
        );
        assert_eq!(
            harness.state().2,
            horizontal,
            "{name}: no sideways movement"
        );
        harness.key_press(if at_tail {
            egui::Key::ArrowDown
        } else {
            egui::Key::ArrowUp
        });
        harness.run();
        assert!(
            (harness.state().1 - initial).abs() < 1.0,
            "{name}: reverse arrow"
        );
        harness
            .state_mut()
            .0
            .layout
            .focus_pane(1)
            .expect("focus files");
        harness.key_press(if at_tail {
            egui::Key::PageUp
        } else {
            egui::Key::PageDown
        });
        harness.run();
        assert!(
            (harness.state().1 - initial).abs() < 1.0,
            "{name}: inactive preview"
        );
        if at_tail {
            harness
                .state_mut()
                .0
                .layout
                .focus_pane(pane)
                .expect("focus preview");
            harness.run();
            let filtered_row_offset = harness.state().1;
            harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .click();
            harness.run();
            harness.key_press(egui::Key::PageUp);
            harness.run();
            harness.key_press(egui::Key::ArrowUp);
            harness.run();
            harness.key_press(egui::Key::Home);
            harness.run();
            harness.key_press(egui::Key::End);
            harness.run();
            harness.key_press(egui::Key::ArrowRight);
            harness.run();
            assert!(
                (harness.state().1 - filtered_row_offset).abs() < 1.0,
                "search keeps keyboard input"
            );
            harness.get_by_label("Time").click();
            harness.run();
            harness.key_press(egui::Key::PageUp);
            harness.run();
            harness.key_press(egui::Key::Home);
            harness.run();
            assert!(
                (harness.state().1 - filtered_row_offset).abs() < 1.0,
                "time picker keeps keyboard input"
            );
            harness.get_by_label("Cancel").click();
            harness.run();
            harness
                .state_mut()
                .0
                .dispatch(
                    "preview.log.follow",
                    &serde_json::json!({
                        "pane": pane, "on": true
                    }),
                )
                .expect("enable live tail");
            harness.key_press(egui::Key::PageUp);
            harness.run();
            assert!(
                harness.state().1 < filtered_row_offset - 200.0,
                "page up leaves the tail"
            );
            assert!(
                !harness
                    .state()
                    .0
                    .preview_slot(pane)
                    .expect("slot")
                    .log()
                    .expect("log")
                    .follow
            );
        } else {
            harness
                .state_mut()
                .0
                .layout
                .focus_pane(pane)
                .expect("focus preview");
            for _ in 0..40 {
                harness.key_press(egui::Key::PageDown);
                harness.run();
            }
            let bottom = harness.state().1;
            assert!(bottom > 5000.0, "page down reaches the end");
            harness.key_press(egui::Key::Home);
            harness.run();
            assert_eq!(harness.state().1, 0.0, "Home jumps to top");
            harness.key_press(egui::Key::End);
            harness.run();
            assert!(
                (harness.state().1 - bottom).abs() < 1.0,
                "End jumps to bottom"
            );
            harness.key_press(egui::Key::PageDown);
            harness.run();
            assert_eq!(harness.state().1, bottom, "offset clamps at the end");
            for _ in 0..40 {
                harness.key_press(egui::Key::PageUp);
                harness.run();
            }
            assert_eq!(harness.state().1, 0.0, "offset clamps at the beginning");
        }
        harness.key_press(egui::Key::End);
        harness.run();
        let bottom = harness.state().1;
        assert!(bottom > 5000.0, "{name}: End reaches bottom");
        harness.key_press(egui::Key::Home);
        harness.run();
        assert_eq!(harness.state().1, 0.0, "{name}: Home reaches top");
        harness.key_press(egui::Key::End);
        harness.run();
        assert!(
            (harness.state().1 - bottom).abs() < 1.0,
            "{name}: End returns exactly"
        );
    }
}

#[test]
fn selected_previews_offer_reload_only_after_source_changes() {
    use egui_kittest::kittest::Queryable;

    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for grid in [false, true] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let second = if grid {
                ("b.png", TINY_PNG)
            } else {
                ("b.log", b"2024-05-01 10:00:00 INFO second\n".as_slice())
            };
            let (mut session, pane) = multi_preview_fixture(&[
                ("a.log", b"2024-05-01 11:00:00 ERROR original\n"),
                second,
            ]);
            assert!(!session.preview_slot(pane).expect("slot").updates_available);
            let fs = MemFs::new();
            fs.mkdirs_str("/logs").expect("mkdir");
            fs.put_str(
                "/logs/a.log",
                b"2024-05-01 11:00:00 ERROR changed-content\n",
            )
            .expect("changed source");
            fs.put_str(&format!("/logs/{}", second.0), second.1)
                .expect("other source");
            session.backends.register(Arc::new(fs));
            let mut harness = Harness::builder()
                .renderer(renderer)
                .with_size(egui::vec2(1000.0, 420.0))
                .with_accessibility_check(false)
                .build_ui_state(
                    move |ui, session: &mut Session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        session.poll();
                        fm_ui::preview::show(ui, session, pane, &tokens);
                    },
                    session,
                );
            assert!(harness.query_by_label("Reload").is_none());
            if !grid {
                harness.get_by_label("Refresh");
                assert!(harness.query_by_label("Live tail").is_none());
            }
            harness
                .state_mut()
                .refresh_read(VfsUri::parse("/logs").expect("uri"))
                .expect("refresh");
            let deadline = Instant::now() + Duration::from_secs(5);
            while !harness
                .state()
                .preview_slot(pane)
                .expect("slot")
                .updates_available
            {
                harness.run();
                assert!(Instant::now() < deadline, "change notification");
                std::thread::sleep(Duration::from_millis(5));
            }
            harness.run();
            harness.get_by_label("2024-05-01 11:00:00 ERROR original");
            harness.snapshot(format!("preview_reload_{grid}_{theme}"));
            harness
                .get_by_label(if grid { "Reload" } else { "Refresh" })
                .click();
            harness.run();
            while harness
                .query_by_label("2024-05-01 11:00:00 ERROR changed-content")
                .is_none()
            {
                harness.run();
                assert!(Instant::now() < deadline, "reloaded content");
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(harness.query_by_label("Reload").is_none());
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn selected_logs_refresh_without_notification_preserves_options() {
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    let (mut session, pane) = multi_preview_fixture(&[
        ("a.log", b"2024-05-01 11:00:00 ERROR original a\n"),
        ("b.log", b"2024-05-01 10:00:00 ERROR original b\n"),
    ]);
    for (id, params) in [
        (
            "preview.log.search",
            json!({"query": "ERROR", "regex": true}),
        ),
        (
            "preview.log.time_range",
            json!({"from": "2024-05-01 09:00", "to": ""}),
        ),
        ("preview.log.level", json!({"level": "error"})),
        ("preview.log.order", json!({"mode": "chronological"})),
        ("preview.log.fold", json!({"mode": "expand"})),
    ] {
        session.dispatch(id, &params).expect("option");
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while session
        .preview_slot(pane)
        .and_then(|s| s.log())
        .is_none_or(|v| v.is_busy())
    {
        session.poll();
        assert!(Instant::now() < deadline, "filter settled");
        std::thread::sleep(Duration::from_millis(5));
    }
    let filter = session
        .preview_slot(pane)
        .expect("slot")
        .log()
        .expect("log")
        .filter
        .clone();
    let fs = MemFs::new();
    fs.mkdirs_str("/logs").expect("mkdir");
    for (name, text) in [
        ("a.log", "2024-05-01 11:30:00 ERROR refreshed a\n"),
        ("b.log", "2024-05-01 11:15:00 ERROR refreshed b\n"),
    ] {
        fs.put_str(&format!("/logs/{name}"), text.as_bytes())
            .expect("changed source");
    }
    session.backends.register(Arc::new(fs));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1050.0, 500.0))
        .with_accessibility_check(false)
        .build_ui_state(
            move |ui, session: &mut Session| {
                session.poll();
                fm_ui::preview::show(ui, session, pane, &Tokens::dark());
            },
            session,
        );
    assert!(
        !h.state()
            .preview_slot(pane)
            .expect("slot")
            .updates_available
    );
    assert!(h.query_by_label("Live tail").is_none());
    h.get_by_label("Refresh").click();
    h.run();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        h.run();
        let view = h.state().preview_slot(pane).expect("slot").log();
        if view.is_some_and(|v| {
            !v.is_busy()
                && v.filtered.is_some()
                && v.doc
                    .read()
                    .expect("doc")
                    .records
                    .iter()
                    .all(|r| r.text.contains("refreshed"))
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "refresh finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    let v = h
        .state()
        .preview_slot(pane)
        .expect("slot")
        .log()
        .expect("log");
    assert_eq!(v.filter, filter);
    assert!(v.chronological && v.expand_all && !v.follow);
    assert_eq!(v.filtered.as_ref().expect("filtered").len(), 2);
    assert_eq!(v.sources.len(), 2);
    assert!(
        h.get_by_label("2024-05-01 11:15:00 ERROR refreshed b")
            .rect()
            .top()
            < h.get_by_label("2024-05-01 11:30:00 ERROR refreshed a")
                .rect()
                .top()
    );
    h.get_by_label("Refresh");
}

#[test]
fn selected_previews_page_the_outer_document_or_grid() {
    let log: String = (0..400)
        .map(|i| format!("2024-05-01 10:00:00 INFO line {i}\n"))
        .collect();
    for grid in [false, true] {
        let files: Vec<_> = (0..9)
            .map(|i| {
                if grid && i > 0 {
                    (format!("{i}.png"), TINY_PNG)
                } else {
                    (format!("{i}.log"), log.as_bytes())
                }
            })
            .collect();
        let entries: Vec<_> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), *bytes))
            .collect();
        let (mut session, pane) = multi_preview_fixture(&entries);
        session.layout.focus_pane(pane).expect("focus preview");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 360.0))
            .with_max_steps(16)
            .with_accessibility_check(false)
            .build_ui_state(
                |ui, (session, offset): &mut (Session, f32)| {
                    fm_ui::preview::show(ui, session, pane, &Tokens::dark());
                    let salt = if grid { "pv-multi-grid" } else { "pv-log" };
                    let id = ui.make_persistent_id(egui::IdSalt::new((salt, pane)));
                    *offset = egui::scroll_area::State::load(ui.ctx(), id)
                        .expect("outer scroll state")
                        .clamped_offset()
                        .y;
                },
                (session, 0.0),
            );
        let initial = harness.state().1;
        harness.key_press(if grid {
            egui::Key::PageDown
        } else {
            egui::Key::PageUp
        });
        harness.run();
        assert!(
            (harness.state().1 - initial).abs() > 200.0,
            "page outer content, grid={grid}"
        );
        harness.key_press(if grid {
            egui::Key::PageUp
        } else {
            egui::Key::PageDown
        });
        harness.run();
        assert!(
            (harness.state().1 - initial).abs() < 1.0,
            "reverse paging, grid={grid}"
        );
        harness.key_press(egui::Key::Home);
        harness.run();
        assert_eq!(
            harness.state().1,
            0.0,
            "Home reaches outer top, grid={grid}"
        );
        harness.key_press(egui::Key::End);
        harness.run();
        assert!(
            harness.state().1 > 300.0,
            "End reaches outer bottom, grid={grid}"
        );
    }
}

#[test]
fn preview_toolbar_keeps_controls_on_one_row_and_logs_have_only_filter() {
    use egui_kittest::kittest::Queryable;
    for tokens in [Tokens::dark(), Tokens::light()] {
        for width in [1000.0, 360.0, 220.0] {
            for log in [false, true] {
                let (mut s, pane) = if log {
                    preview_fixture(
                        "long-log-filename.log",
                        b"2026-10-30T12:00:00Z INFO ready\n",
                        false,
                    )
                } else {
                    preview_fixture(
                        "very-long-markdown-preview-filename.md",
                        b"# Title\n\nText",
                        false,
                    )
                };
                s.previews.slots.get_mut(&pane).expect("slot").scale = 1.44;
                let mut harness = Harness::builder()
                    .with_size(egui::vec2(width, 420.0))
                    .with_accessibility_check(false)
                    .build_ui_state(
                        move |ui, s: &mut Session| {
                            theme::apply_to(ui.ctx(), &tokens);
                            fm_ui::preview::show(ui, s, pane, &tokens);
                        },
                        s,
                    );
                harness.run();
                let source = harness.get_by_label("Preview source").rect();
                if log {
                    let filter = harness.get_by_label("Filter").rect();
                    assert!((filter.center().y - source.center().y).abs() < 2.0);
                    assert!(harness.query_by_label("Find").is_none());
                }
                if width == 1000.0 {
                    let zoom = harness.get_by_label("Reset preview zoom").rect();
                    assert!((zoom.center().y - source.center().y).abs() < 2.0);
                    assert!(zoom.right() <= width);
                    if !log {
                        let find = harness.get_by_label("Find").rect();
                        assert!((find.center().y - source.center().y).abs() < 2.0);
                        harness.get_by_label("Find").click();
                        harness.run();
                        harness.get_by_label("Find in preview");
                    }
                } else {
                    let menu = harness.get_by_label("View options").rect();
                    assert!((menu.center().y - source.center().y).abs() < 2.0);
                    assert!(
                        menu.right() <= width,
                        "overflow stays within narrow toolbar"
                    );
                    assert!(harness.query_by_label("Reset preview zoom").is_none());
                    harness.get_by_label("View options").click();
                    harness.run();
                    harness.get_by_label("Reset preview zoom").click();
                    harness.run();
                    assert_eq!(harness.state().preview_slot(pane).expect("slot").scale, 1.0);
                    if log {
                        assert!(harness.query_by_label("Find").is_none());
                    } else {
                        // Menu actions may leave the menu open.
                        if harness.query_by_label("Find").is_none() {
                            harness.get_by_label("View options").click();
                            harness.run();
                        }
                        harness.get_by_label("Find").click();
                        harness.run();
                        harness.get_by_label("Find in preview");
                    }
                }
            }
        }
    }
}

#[test]
fn preview_toolbar_folder_path_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (mut session, pane) = preview_fixture(
            "a-very-long-preview-filename-with-many-characters.md",
            b"# Preview\n\nThe toolbar identifies this file's folder.",
            false,
        );
        session
            .previews
            .slots
            .get_mut(&pane)
            .expect("slot")
            .subject
            .as_mut()
            .expect("subject")
            .uri = VfsUri::parse(
            "/home/alice/Pictures/Screenshots/a-very-long-preview-filename-with-many-characters.md",
        )
        .expect("uri");
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_accessibility_check(false)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::preview::show(ui, &mut session, pane, &tokens);
            });
        for width in [1400, 800, 360] {
            harness.set_size(egui::vec2(width as f32, 180.0));
            harness.run();
            harness.snapshot(format!("preview_folder_path_{width}_{theme}"));
        }
        results.extend_harness(&mut harness);
    }
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
fn preview_filter_row_toggles_and_preserves_settings_dark_light() {
    use egui_kittest::kittest::{NodeT, Queryable};

    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for width in [1000, 300] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let (session, pane) = multi_preview_fixture(&[
                ("a.log", b"2024-05-01 10:00:00 INFO started\n"),
                ("b.log", b"2024-05-01 11:00:00 ERROR failed\n"),
            ]);
            let mut harness = Harness::builder()
                .renderer(renderer)
                .with_size(egui::vec2(width as f32, 420.0))
                .with_accessibility_check(false)
                .build_ui_state(
                    move |ui, session: &mut Session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        fm_ui::preview::show(ui, session, pane, &tokens);
                    },
                    session,
                );
            assert_eq!(
                harness.query_all_by_label(".*").count(),
                1,
                "initially shown"
            );
            harness.get_by_label("Filter").click();
            harness.run();
            assert_eq!(harness.query_all_by_label(".*").count(), 0);
            harness.snapshot(format!("preview_filters_closed_{width}_{theme}"));
            harness.get_by_label("Filter").click();
            harness.run();
            let button = harness.get_by_label("Filter").rect();
            let selector = harness
                .get_all_by_role(egui::accesskit::Role::ComboBox)
                .next()
                .expect("panel selector")
                .rect();
            assert!(
                // The Preview/Diff toggle of a two-file selection sits between them.
                button.left() >= selector.right() && button.left() - selector.right() < 160.0,
                "Filter follows the panel selector"
            );
            let clear = harness.get_by_label("Clear").rect();
            let mark = harness.get_by_label("Mark").rect();
            assert!(
                clear.top() > mark.top()
                    || (clear.top() == mark.top() && clear.left() > mark.left()),
                "Clear is the last control"
            );
            assert!(
                button.right() <= width as f32,
                "Filter button stays visible"
            );
            for label in [
                "Preview order",
                "Minimum severity",
                ".*",
                "Time",
                "Expand",
                "Mark",
            ] {
                let rect = harness.get_by_label(label).rect();
                assert!(
                    rect.top() >= button.bottom(),
                    "{label} belongs below the main toolbar"
                );
                assert!(rect.right() <= width as f32, "{label} fits narrow panes");
            }
            harness.get_by_label(".*").click();
            harness.run();
            assert!(
                harness
                    .state()
                    .preview_slot(pane)
                    .expect("slot")
                    .log()
                    .expect("log")
                    .filter
                    .regex
            );
            harness.snapshot(format!("preview_filters_open_{width}_{theme}"));
            harness.get_by_label("Filter").click();
            harness.run();
            assert_eq!(harness.query_all_by_label(".*").count(), 0);
            assert!(
                harness
                    .state()
                    .preview_slot(pane)
                    .expect("slot")
                    .log()
                    .expect("log")
                    .filter
                    .regex
            );
            harness.get_by_label("Filter").click();
            harness.run();
            assert_eq!(
                harness.get_by_label(".*").accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::True)
            );
            harness
                .get_by_role(egui::accesskit::Role::TextInput)
                .type_text("failed");
            harness.run();
            for (command, params) in [
                (
                    "preview.log.level",
                    serde_json::json!({"pane": pane, "level": "error"}),
                ),
                (
                    "preview.log.time_range",
                    serde_json::json!({"pane": pane, "from": "2024-05-01 10:30:00", "to": "2024-05-01 12:00:00"}),
                ),
            ] {
                harness
                    .state_mut()
                    .dispatch(command, &params)
                    .expect("configure filters");
            }
            harness.run();
            harness.get_by_label("Clear").click();
            harness.run();
            assert!(
                harness
                    .state()
                    .preview_slot(pane)
                    .expect("slot")
                    .log()
                    .expect("log")
                    .filter
                    .is_empty()
            );
            assert_eq!(
                harness.get_by_label(".*").accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::False)
            );
            assert!(harness.get_by_label("Clear").accesskit_node().is_disabled());
            assert!(
                harness
                    .get_by_role(egui::accesskit::Role::TextInput)
                    .accesskit_node()
                    .value()
                    .unwrap_or_default()
                    .is_empty()
            );
            harness.get_by_label(".*").click();
            harness.run();
            harness
                .state_mut()
                .dispatch(
                    "preview.log.clear_filters",
                    &serde_json::json!({"pane": pane}),
                )
                .expect("menu clear");
            harness.run();
            assert_eq!(
                harness.get_by_label(".*").accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::False)
            );
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn log_time_picker_dark_light() {
    use egui_kittest::kittest::{NodeT, Queryable};

    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (session, pane) = preview_fixture(
            "app.log",
            b"[2024-05-01 10:00:00] main.INFO: started\n[2024-05-02 11:30:45] main.ERROR: failed\n",
            false,
        );
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_size(egui::vec2(900.0, 650.0))
            .with_accessibility_check(false)
            .build_ui_state(
                move |ui, session: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    ui.set_max_width(300.0);
                    fm_ui::preview::show(ui, session, pane, &tokens);
                },
                session,
            );
        harness.get_by_label("Time").click();
        harness.run();
        harness.get_by_label("From").click();
        harness.run();
        harness.get_by_label("To").click();
        harness.run();
        harness.get_by_label("From time").click();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.run();
        harness.get_by_label("From time").type_text("09:00");
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness.get_by_label("Log time range");
        harness.snapshot(format!("log_time_picker_{theme}"));
        harness
            .get_all_by_label("Calendar")
            .next()
            .expect("From calendar")
            .click();
        harness.run();
        harness.get_by_label("Save");
        harness.snapshot(format!("log_time_calendar_{theme}"));
        harness.get_by_label("Save").click();
        harness.run();
        harness.get_by_label("Apply").click();
        harness.run();
        let filter = &harness
            .state()
            .preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .filter;
        assert_eq!(
            filter.from,
            fm_preview::log::parse_bound("2024-05-01 09:00:00")
        );
        assert_eq!(
            filter.to,
            fm_preview::log::parse_bound("2024-05-02 11:30:45")
        );
        harness.get_by_label("Time").click();
        harness.run();
        for label in ["From", "To"] {
            assert_eq!(
                harness.get_by_label(label).accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::True)
            );
        }
        harness
            .get_by_label("Log time range")
            .get_by_label("Clear")
            .click();
        harness.run();
        let filter = &harness
            .state()
            .preview_slot(pane)
            .expect("slot")
            .log()
            .expect("log")
            .filter;
        assert!(filter.from.is_none() && filter.to.is_none());
        results.extend_harness(&mut harness);
    }
}

#[test]
fn saved_servers_pane_dark_light() {
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
            protocol: fm_engine::prefs::Protocol::Sftp,
            domain: None,
            share: None,
        });
        session.prefs.profiles.push(fm_engine::prefs::Profile {
            name: "build box".into(),
            host: "10.0.0.5".into(),
            port: 2222,
            user: "ci".into(),
            key_path: Some("/home/alice/.ssh/id_ed25519".into()),
            start_dir: None,
            protocol: fm_engine::prefs::Protocol::Sftp,
            domain: None,
            share: None,
        });
        session.layout.state_mut(1).expect("pane").kind = fm_layout::PaneKind::Sftp;
        session.open_servers(1).expect("servers");
        wait_ready(&mut session, &VfsUri::servers());
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            let _ = pane::show(ui, &mut session, 1, &tokens);
        });
        harness.set_size(egui::vec2(640.0, 460.0));
        harness.run();
        harness.snapshot(format!("pane_saved_servers_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn sftp_popup_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.remote.sftp_prompt = Some(fm_engine::remote::SftpPrompt {
            navigate: true,
            host: "nas.local".into(),
            user: "alice".into(),
            key_path: "~/.ssh/id_rsa".into(),
            ..Default::default()
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(480.0, 480.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::sftp_popup::sftp_prompt_dialog(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("sftp_popup_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn sftp_unreachable_popup_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.remote.sftp_prompt = Some(fm_engine::remote::SftpPrompt {
            host: "192.168.0.50".into(),
            user: "alice".into(),
            error: Some("Cannot reach the SFTP server. Check the server address, network or VPN, and firewall.".into()),
            ..Default::default()
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(540.0, 480.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::sftp_popup::sftp_prompt_dialog(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("sftp_unreachable_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn login_dialog_dark_light() {
    use fm_engine::network::LoginPrompt;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.remote.login_prompt = Some(LoginPrompt {
            uri: VfsUri::parse("smb://nas.local/").expect("uri"),
            host: "nas.local".to_string(),
            port: None,
            error: Some("Login failed. Check the user name, domain and password.".to_string()),
            user: "alice".to_string(),
            domain: "WORKGROUP".to_string(),
            navigate: false,
            manual: false,
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 400.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::remote::login_dialog(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("login_dialog_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn manual_connect_dialog_dark_light() {
    use fm_engine::network::LoginPrompt;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = fixture_session();
        session.remote.login_prompt = Some(LoginPrompt {
            uri: VfsUri::parse("network:///").expect("uri"),
            host: String::new(),
            port: None,
            error: None,
            user: String::new(),
            domain: "WORKGROUP".to_string(),
            navigate: false,
            manual: true,
        });
        let mut harness = Harness::builder()
            .with_size(egui::vec2(640.0, 600.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::remote::login_dialog(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("network_connect_dialog_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn network_pane_dark_light() {
    use fm_discovery::{HostList, NetworkFs, Service};
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let hosts = HostList::new();
        for (name, ip, svc, port) in [
            ("nas", "192.168.1.20", Service::Smb, 445),
            ("nas", "192.168.1.20", Service::Sftp, 22),
            ("media-pc", "192.168.1.31", Service::Smb, 445),
        ] {
            hosts.upsert(name, name, ip.parse().expect("ip"), svc, port);
        }
        let mut session = test_session();
        session.backends.register(Arc::new(NetworkFs::fixed(hosts)));
        let net = VfsUri::network();
        {
            let st = session.layout.state_mut(1).expect("pane");
            st.kind = fm_layout::PaneKind::Network;
            st.tree = fm_layout::TreeState::new(net.clone());
            st.navigate(net.clone());
        }
        session.submit_read(net.clone()).expect("submit");
        wait_ready(&mut session, &net);
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            let _ = pane::show(ui, &mut session, 1, &tokens);
        });
        harness.set_size(egui::vec2(640.0, 360.0));
        harness.run();
        harness.snapshot(format!("pane_network_{theme}"));
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
        let mut session = test_session();
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
        let mut session = test_session();
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
                session
                    .remote
                    .set_state_for_tests(uri.scheme(), uri.authority(), state);
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
        let mut harness = Harness::builder()
            .with_size(egui::vec2(560.0, 360.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::remote::host_key_alert(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("dialog_hostkey_changed_{theme}"));
        results.extend_harness(&mut harness);
    }
}

/// The fixture plus the command registry, for tests that dispatch.
fn commanded_fixture() -> Session {
    let mut s = fixture_session();
    s.registry = fm_engine::Registry::from_specs(fm_engine::commands::specs()).expect("registry");
    s
}

fn wait_until(s: &mut Session, mut done: impl FnMut(&Session) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(s) {
        s.poll();
        assert!(Instant::now() < deadline, "background work never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn filter_bar_and_places_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = commanded_fixture();
        session
            .dispatch(
                "bookmark.add",
                &serde_json::json!({"uri": "/home/alice/docs", "name": "Docs"}),
            )
            .expect("bookmark");
        session
            .dispatch("view.filter", &serde_json::json!({"text": "o"}))
            .expect("filter");
        let mut harness = Harness::builder().renderer(renderer).build_ui(move |ui| {
            theme::apply_to(ui.ctx(), &tokens);
            ui.ctx().set_theme(if tokens.dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            });
            let _ = pane::show(ui, &mut session, 1, &tokens);
        });
        harness.set_size(egui::vec2(640.0, 400.0));
        harness.run();
        harness.snapshot(format!("pane_filter_places_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn volume_preview_dark_light() {
    use egui_kittest::kittest::Queryable;
    use fm_engine::devices::{Device, Removal};
    use fm_engine::preview_cmds::Body;
    use fm_preview::Preview;

    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for width in [420, 280] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let filename = "A descriptive filename that should wrap inside the preview.txt";
            let (mut session, pane) = preview_fixture(filename, b"example", false);
            let fs = MemFs::new();
            fs.mkdirs_str("/home/alice").expect("root");
            fs.put_str(&format!("/home/alice/{filename}"), b"example")
                .expect("file");
            for name in [
                "Projects",
                "Photos",
                "A long subfolder name that should stay on one line",
            ] {
                fs.mkdirs_str(&format!("/home/alice/{name}"))
                    .expect("folder");
                fs.put_str(&format!("/home/alice/{name}/sample.txt"), b"sample")
                    .expect("child");
            }
            session.backends.register(Arc::new(fs));
            session.prefs.recursive_preview = true;
            session
                .layout
                .state_mut(1)
                .expect("files pane")
                .listing
                .on_current = true;
            let volume = Device {
                name: "USB backup".into(),
                mount: VfsUri::parse("/home/alice").expect("mount"),
                filesystem: "exfat".into(),
                total: Some(128 << 30),
                free: Some(96 << 30),
                identity: "usb-42".into(),
                removal: Removal::Eject,
            };
            session.volumes.replace(vec![
                Device {
                    name: "Outer volume".into(),
                    mount: VfsUri::parse("/home").expect("outer mount"),
                    ..volume.clone()
                },
                volume.clone(),
            ]);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                session.poll();
                if session.preview_slot(pane).is_some_and(|slot| {
                    matches!(&slot.body, Body::Static(p) if matches!(&**p, Preview::Dir(d) if !d.scanning))
                }) {
                    break;
                }
                assert!(Instant::now() < deadline, "directory preview never settled");
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut h = Harness::builder()
                .renderer(renderer)
                .with_accessibility_check(false)
                .with_size(egui::vec2(width as f32, 900.0))
                .build_ui_state(
                    move |ui, session| {
                        theme::apply_to(ui.ctx(), &tokens);
                        ui.ctx().set_theme(if tokens.dark {
                            egui::Theme::Dark
                        } else {
                            egui::Theme::Light
                        });
                        fm_ui::preview::show(ui, session, pane, &tokens);
                    },
                    session,
                );
            h.run();
            h.get_by_label("Volume");
            h.get_by_label("USB backup");
            assert!(h.query_by_label("Outer volume").is_none());
            assert_eq!(h.query_all_by_label("/home/alice").count(), 2);
            h.get_by_label("96.0 GiB");
            h.get_by_label("Folder summary");
            h.get_by_label("Subfolders");
            let first = h.get_by_label("Projects/").rect();
            let second = h.get_by_label("Photos/").rect();
            assert!((first.top() - second.top()).abs() <= tokens.font_size + 10.0);
            let long = h
                .get_by_label("A long subfolder name that should stay on one line/")
                .rect();
            assert!(long.height() <= tokens.font_size + 4.0);
            assert!(long.right() <= width as f32);
            if width >= 420 {
                assert!(long.width() > width as f32 * 0.5);
            }
            assert!(h.query_all_by_label(filename).count() > 0);
            for label in h.query_all_by_label(filename) {
                assert!(label.rect().right() <= width as f32);
                assert!(label.rect().bottom() <= h.get_by_label("Scope").rect().top());
            }
            h.snapshot(format!("volume_preview_{width}_{theme}"));
            let mut unknown = volume;
            unknown.free = None;
            h.state_mut().volumes.replace(vec![unknown]);
            h.run();
            h.get_by_label("Unavailable");
            h.state_mut()
                .dispatch("preview.thumbs.toggle", &serde_json::json!({"pane": pane}))
                .expect("thumbnails");
            h.run();
            h.get_by_label("Volume information").click();
            h.run();
            h.get_by_label("USB backup");
            h.get_by_label("Unavailable");
            h.state_mut()
                .dispatch("preview.thumbs.toggle", &serde_json::json!({"pane": pane}))
                .expect("summary");
            h.state_mut().volumes.replace(Vec::new());
            h.run();
            assert!(h.query_by_label("Volume").is_none());
            h.get_by_label("Folder summary");
            let before = h.get_by_label("Folder summary").rect().height();
            let global = h.state().prefs.zoom;
            if h.query_by_label("+").is_none() {
                h.get_by_label("View options").click();
                h.run();
            }
            h.get_by_label("+").click();
            h.run();
            h.key_press(egui::Key::Escape);
            h.run();
            assert!(h.get_by_label("Folder summary").rect().height() > before * 1.1);
            assert_eq!(h.state().prefs.zoom, global);
            h.state_mut()
                .dispatch("preview.thumbs.toggle", &serde_json::json!({"pane": pane}))
                .expect("thumbnails");
            h.run();
            if h.query_by_label("+").is_none() {
                h.get_by_label("View options").click();
                h.run();
            }
            h.get_by_label("+").click();
            h.run();
            h.key_press(egui::Key::Escape);
            h.run();
            assert!(h.state().preview_slot(pane).expect("slot").scale > 1.4);
            assert_eq!(h.state().prefs.zoom, global);
            results.extend_harness(&mut h);
        }
    }
}

#[test]
fn devices_section_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let pale_accent = fm_engine::Prefs {
        theme: fm_engine::ThemePreference::Light,
        accent: Some("#68b7dc".into()),
        ..Default::default()
    };
    for (tokens, theme) in [
        (Tokens::dark(), "dark"),
        (Tokens::light(), "light"),
        (
            Tokens::resolve_with(&pale_accent, false),
            "light_pale_accent",
        ),
    ] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = commanded_fixture();
        session.volumes.replace(vec![
            fm_engine::devices::Device {
                name: "USB backup".into(),
                mount: VfsUri::parse("/media/Backup").expect("uri"),
                filesystem: "exfat".into(),
                total: Some(128 << 30),
                free: Some(96 << 30),
                identity: "usb-42".into(),
                removal: fm_engine::devices::Removal::Eject,
            },
            fm_engine::devices::Device {
                name: "Private files".into(),
                mount: VfsUri::parse("/media/veracrypt1").expect("uri"),
                filesystem: "ext4".into(),
                total: Some(16 << 30),
                free: Some(8 << 30),
                identity: "virtual-42".into(),
                removal: fm_engine::devices::Removal::Unsupported(
                    "Dismount this volume in VeraCrypt".into(),
                ),
            },
        ]);
        let mut h = Harness::builder()
            .with_size(egui::vec2(400.0, 900.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                let _ = fm_ui::places::show(ui, &mut session, 1, &tokens);
            });
        h.run_steps(3);
        h.snapshot(format!("devices_section_{theme}"));
        results.extend_harness(&mut h);
    }
}

#[test]
fn the_filter_bar_takes_typing_and_esc_clears_it() {
    let tokens = Tokens::dark();
    let mut fixture = commanded_fixture();
    fixture
        .dispatch("view.filter", &serde_json::Value::Null)
        .expect("open the bar");
    let session = std::rc::Rc::new(std::cell::RefCell::new(fixture));
    let shared = session.clone();
    let mut harness = Harness::builder().build_ui(move |ui| {
        let _ = pane::show(ui, &mut shared.borrow_mut(), 1, &tokens);
    });
    harness.set_size(egui::vec2(640.0, 400.0));
    harness.run();
    harness.event(egui::Event::Text("no".to_string()));
    harness.run();
    let filter = |s: &Session| s.layout.state(1).and_then(|p| p.listing.filter.clone());
    assert_eq!(filter(&session.borrow()).as_deref(), Some("no"));
    let (_, ix) = session.borrow_mut().pane_view(1).expect("view");
    assert_eq!(ix.order.len(), 1, "only notes.txt matches");
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(filter(&session.borrow()), None);
}

#[test]
fn properties_window_counts_a_folder_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let mut session = commanded_fixture();
        session
            .dispatch(
                "file.properties",
                &serde_json::json!({"sources": ["/home/alice"]}),
            )
            .expect("properties");
        wait_until(&mut session, |s| {
            s.properties.as_ref().is_some_and(|p| p.info().done)
        });
        let info = session.properties.as_ref().expect("open").info();
        assert_eq!((info.files, info.dirs, info.bytes), (2, 3, 14 + 4096));
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(520.0, 420.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::properties::show(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("dialog_properties_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn find_files_window_lists_results_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let mut session = commanded_fixture();
        session
            .dispatch("search.open", &serde_json::json!({"root": "/home/alice"}))
            .expect("open");
        session
            .dispatch("search.start", &serde_json::json!({"name": "*.txt; *.bin"}))
            .expect("start");
        wait_until(&mut session, |s| {
            s.search.as_ref().is_some_and(|x| !x.status().running)
        });
        assert_eq!(session.search.as_ref().expect("open").status().found, 2);
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 480.0))
            .renderer(renderer)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                ui.ctx().set_theme(if tokens.dark {
                    egui::Theme::Dark
                } else {
                    egui::Theme::Light
                });
                fm_ui::search::show(ui.ctx(), &mut session, &tokens);
            });
        harness.run();
        harness.run_steps(3);
        harness.snapshot(format!("dialog_find_files_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn preview_diff_split_and_unified_dark_light() {
    use fm_engine::diff_cmds::DiffStatus;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let dir = tempfile::tempdir().expect("tmp");
    let d = dir.path();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(d)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    let file = d.join("main.rs");
    std::fs::write(
        &file,
        "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n    old();\n}\n",
    )
    .expect("old");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "i"]);
    std::fs::write(
        &file,
        "fn main() {\n    let x = 2;\n    println!(\"{x}\");\n    added();\n    more();\n}\n",
    )
    .expect("new");
    let dir_uri = VfsUri::parse(&d.to_string_lossy()).expect("uri");
    let file_uri = VfsUri::parse(&file.to_string_lossy()).expect("uri");
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        for mode in ["split", "unified"] {
            let Some(renderer) = renderer_or_skip() else {
                return;
            };
            let mut session = with_fixed_places(Session::new().expect("session"));
            let files = session.layout.focused();
            session
                .layout
                .state_mut(files)
                .expect("pane")
                .navigate(dir_uri.clone());
            session.ensure_read(dir_uri.clone()).expect("read");
            wait_ready(&mut session, &dir_uri);
            session
                .dispatch(
                    "preview.open",
                    &serde_json::json!({"uri": file_uri.to_string()}),
                )
                .expect("open");
            let pane = session
                .layout
                .panes()
                .into_iter()
                .find(|p| session.pane_kind(*p) == Some(fm_layout::PaneKind::Preview))
                .expect("preview");
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                session.poll();
                if session.pane_diff(pane).is_some_and(|d| d.active)
                    && matches!(
                        session.diff_view(pane).map(|v| &v.status),
                        Some(DiffStatus::Ready(_))
                    )
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            session
                .dispatch("preview.diff.mode", &serde_json::json!({"mode": mode}))
                .expect("mode");
            // The toolbar must not snapshot the random temporary repository path.
            session
                .previews
                .slots
                .get_mut(&pane)
                .expect("slot")
                .subject
                .as_mut()
                .expect("subject")
                .uri = VfsUri::parse("/home/alice/project/main.rs").expect("uri");
            let mut harness = Harness::builder()
                .renderer(renderer)
                .with_accessibility_check(false)
                .build_ui(move |ui| {
                    theme::apply_to(ui.ctx(), &tokens);
                    egui::Frame::NONE.fill(tokens.panel).show(ui, |ui| {
                        fm_ui::preview::show(ui, &mut session, pane, &tokens);
                    });
                });
            harness.set_size(egui::vec2(900.0, 260.0));
            harness.run();
            harness.snapshot(format!("preview_diff_{mode}_{theme}"));
            results.extend_harness(&mut harness);
        }
    }
}

#[test]
fn git_markers_in_listing_dark_light() {
    use fm_engine::git::Mark;
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let dir = tempfile::tempdir().expect("tmp");
    let d = dir.path();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .current_dir(d)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    for f in ["clean.txt", "edited.txt", "removed.txt"] {
        std::fs::write(d.join(f), "1\n").expect("write");
    }
    std::fs::create_dir(d.join("src")).expect("dir");
    std::fs::write(d.join("src/lib.rs"), "1\n").expect("write");
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "i"]);
    std::fs::write(d.join("edited.txt"), "2\n").expect("write");
    std::fs::write(d.join("src/lib.rs"), "2\n").expect("write");
    std::fs::remove_file(d.join("removed.txt")).expect("rm");
    std::fs::write(d.join("new.txt"), "n\n").expect("write");
    std::fs::write(d.join("staged.txt"), "s\n").expect("write");
    git(&["add", "staged.txt"]);
    for entry in std::fs::read_dir(d).expect("dir").flatten() {
        if let Ok(f) = std::fs::File::open(entry.path()) {
            let _ = f.set_modified(FIXED_MTIME + Duration::from_secs(86_400 * 365));
        }
    }
    let uri = VfsUri::parse(&d.to_string_lossy()).expect("uri");
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let mut session = with_fixed_places(Session::new().expect("session"));
        session.layout.state_mut(1).expect("pane").location = uri.clone();
        session.submit_read(uri.clone()).expect("submit");
        wait_ready(&mut session, &uri);
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            session.poll();
            if session
                .git_view(&uri)
                .is_some_and(|st| st.mark_abs(&d.join("edited.txt")) == Some(Mark::Modified))
            {
                break;
            }
            assert!(Instant::now() < deadline, "git status never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut harness = Harness::builder()
            .renderer(renderer)
            .with_accessibility_check(false)
            .build_ui(move |ui| {
                theme::apply_to(ui.ctx(), &tokens);
                egui::Frame::NONE.fill(tokens.panel).show(ui, |ui| {
                    listing::show(ui, &mut session, 1, &tokens, true);
                });
            });
        harness.set_size(egui::vec2(640.0, 300.0));
        harness.run();
        harness.snapshot(format!("listing_git_markers_{theme}"));
        results.extend_harness(&mut harness);
    }
}

#[test]
fn opening_the_preview_source_list_does_not_panic_and_lists_numbered_panes() {
    use egui_kittest::kittest::Queryable;
    let tokens = Tokens::dark();
    let (s, pane) = preview_fixture("a.md", b"# Title", false);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 420.0))
        .with_accessibility_check(false)
        .build_ui_state(
            move |ui, s: &mut Session| {
                theme::apply_to(ui.ctx(), &tokens);
                fm_ui::preview::show(ui, s, pane, &tokens);
            },
            s,
        );
    harness.run();
    harness.get_by_label("Preview source").click();
    harness.run();
    harness.get_by_label_contains("Pane 1");
}

#[test]
fn markdown_images_load_and_replace_their_placeholders() {
    use egui_kittest::kittest::Queryable;
    let doc = "![logo](l.svg)\n\n![hero](p.webp#gh-dark-mode-only)\n\n| a |\n|---|\n| ![cell](p.webp) |\n";
    let (mut s, pane) = preview_fixture_with(
        "doc.md",
        doc.as_bytes(),
        false,
        &[
            (
                "p.webp",
                include_bytes!("../../../docs/screenshots/network-dark.webp"),
            ),
            ("l.svg", include_bytes!("../../../packaging/logo.svg")),
        ],
    );
    s.layout.focus_pane(pane).expect("focus");
    let tokens = Tokens::dark();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(600.0, 400.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                theme::apply_to(ui.ctx(), &tokens);
                fm_ui::preview::show(ui, s, pane, &tokens);
            },
            s,
        );
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(2);
        let placeholders = harness.query_all_by_label_contains("[Image").count();
        if placeholders == 0 || Instant::now() > end {
            assert_eq!(
                placeholders,
                0,
                "images still placeholders; notice {:?}",
                harness.state().notice
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn markdown_html_pictures_follow_the_theme() {
    use egui_kittest::kittest::Queryable;
    let doc = "<h1 align=\"center\">Centered Title</h1>\n\n<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"p.webp\">\n  <img src=\"missing-light.webp\" alt=\"hero\">\n</picture>\n\n<table>\n  <tr>\n    <td>\n      <picture>\n        <source media=\"(prefers-color-scheme: dark)\" srcset=\"p.webp\">\n        <img src=\"missing-light.webp\" alt=\"cell\" width=\"120\">\n      </picture>\n    </td>\n  </tr>\n</table>\n";
    let run = |tokens: Tokens, settle: bool| {
        let (mut s, pane) = preview_fixture_with(
            "doc.md",
            doc.as_bytes(),
            false,
            &[(
                "p.webp",
                include_bytes!("../../../docs/screenshots/network-dark.webp"),
            )],
        );
        s.layout.focus_pane(pane).expect("focus");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 500.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        let start = Instant::now();
        loop {
            harness.run_steps(2);
            let placeholders = harness.query_all_by_label_contains("[Image").count();
            let settled = if settle {
                start.elapsed() > Duration::from_millis(1500)
            } else {
                placeholders == 0 || start.elapsed() > Duration::from_secs(10)
            };
            if settled {
                return (
                    placeholders,
                    harness
                        .query_all_by_label_contains("Centered Title")
                        .count(),
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    let (dark_placeholders, title) = run(Tokens::dark(), false);
    assert_eq!(title, 1, "centered heading text is rendered, not markup");
    assert_eq!(dark_placeholders, 0, "dark theme loads the dark source");
    let (light_placeholders, _) = run(Tokens::light(), true);
    assert_eq!(
        light_placeholders, 2,
        "light theme uses the missing fallback"
    );
}

#[test]
fn markdown_html_readme_dark_light() {
    let _gpu = lock_gpu();
    let mut results = SnapshotResults::new();
    let doc = "<p align=\"center\">\n  <img src=\"l.svg\" alt=\"logo\" width=\"96\">\n</p>\n\n<h1 align=\"center\">Product</h1>\n\n<p align=\"center\">\n  A multi-pane file manager.<br>\n  Native <code>egui</code> UI.\n</p>\n\n<p align=\"center\">\n  <a href=\"a.md\">Website</a> &middot;\n  <a href=\"b.md\">Download</a>\n</p>\n\n<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"n-dark.webp\">\n  <img src=\"n-light.webp\" alt=\"hero\">\n</picture>\n\n<table>\n  <tr>\n    <td>\n      <picture>\n        <source media=\"(prefers-color-scheme: dark)\" srcset=\"n-dark.webp\">\n        <img src=\"n-light.webp\" alt=\"one\">\n      </picture>\n    </td>\n    <td>\n      <picture>\n        <source media=\"(prefers-color-scheme: dark)\" srcset=\"n-dark.webp\">\n        <img src=\"n-light.webp\" alt=\"two\">\n      </picture>\n    </td>\n  </tr>\n</table>\n";
    for (tokens, theme) in [(Tokens::dark(), "dark"), (Tokens::light(), "light")] {
        let Some(renderer) = renderer_or_skip() else {
            return;
        };
        let (mut s, pane) = preview_fixture_with(
            "a-readme.md",
            doc.as_bytes(),
            false,
            &[
                (
                    "n-dark.webp",
                    include_bytes!("../../../docs/screenshots/network-dark.webp"),
                ),
                (
                    "n-light.webp",
                    include_bytes!("../../../docs/screenshots/network-light.webp"),
                ),
                ("l.svg", include_bytes!("../../../packaging/logo.svg")),
            ],
        );
        s.layout.focus_pane(pane).expect("focus");
        let mut harness = Harness::builder()
            .with_size(egui::vec2(700.0, 1000.0))
            .renderer(renderer)
            .build_ui_state(
                move |ui, s: &mut Session| {
                    theme::apply_to(ui.ctx(), &tokens);
                    ui.ctx().set_theme(if tokens.dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    fm_ui::preview::show(ui, s, pane, &tokens);
                },
                s,
            );
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            harness.run_steps(2);
            use egui_kittest::kittest::Queryable;
            let placeholders = harness.query_all_by_label_contains("[Image").count();
            if placeholders == 0 || Instant::now() > end {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        harness.run_steps(4);
        harness.snapshot(format!("markdown_html_readme_{theme}"));
        results.extend_harness(&mut harness);
    }
}
