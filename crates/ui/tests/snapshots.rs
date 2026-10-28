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
            "Connections",
            "Workspaces",
            "Advanced",
        ] {
            harness.get_by_label(topic).click();
            harness.run();
            harness.run_steps(3);
            harness.snapshot(format!(
                "dialog_preferences_{}_{theme}",
                topic.to_lowercase().replace(' ', "_")
            ));
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
    let mem = MemFs::new();
    mem.mkdirs_str("/home/alice").expect("mkdirs");
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
        harness.get_by_label("Filter").click();
        harness.run();
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
            harness.get_by_label("Filter").click();
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
                harness.get_by_label("Filter").click();
                harness.run();
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
    h.get_by_label("Filter").click();
    h.run();
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
                0,
                "initially hidden"
            );
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
                button.left() >= selector.right() && button.left() - selector.right() < 20.0,
                "Filter is next to the panel selector"
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
        harness.get_by_label("Filter").click();
        harness.run();
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
