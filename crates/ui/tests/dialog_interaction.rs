#![allow(clippy::expect_used)]

use egui::{Context, Id};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use fm_engine::{Session, remote::SftpPrompt};
use fm_ui::{
    Tokens,
    dialogs::{Dialog, Dialogs},
};
use fm_vfs::VfsUri;
use serde_json::json;

fn drag<T>(h: &mut Harness<'_, T>, id: Id) {
    h.run_steps(3);
    let before = h.ctx.memory(|m| m.area_rect(id)).expect("window");
    assert!(
        before.center().distance(h.ctx.content_rect().center()) < 2.0,
        "initially centered: {before:?}"
    );
    let start = egui::pos2(before.center().x, before.top() + 16.0);
    let delta = egui::vec2(60.0, 35.0);
    h.hover_at(start);
    h.run_steps(3);
    h.drag_at(start);
    h.run_steps(3);
    h.hover_at(start + delta);
    h.run_steps(3);
    h.drop_at(start + delta);
    h.run_steps(3);
    let after = h.ctx.memory(|m| m.area_rect(id)).expect("dragged window");
    assert!(
        (after.min - before.min - delta).length() < 2.0,
        "{before:?} -> {after:?}"
    );
    h.run_steps(3);
    let settled = h.ctx.memory(|m| m.area_rect(id)).expect("settled window");
    assert!(
        (settled.min - after.min).length() < 1.0,
        "position persists"
    );
}

#[test]
fn operation_dialogs_drag_and_title_close_cancels_without_dispatch() {
    let root = VfsUri::root_file();
    for tokens in [Tokens::dark(), Tokens::light()] {
        for dialog in [
            Dialog::Mkdir {
                dir: root.clone(),
                name: "folder".into(),
            },
            Dialog::NewFile {
                dir: root.clone(),
                name: "file".into(),
            },
            Dialog::Rename {
                dir: root.clone(),
                current: "old".into(),
                name: "new".into(),
            },
            Dialog::RenameWorkspace {
                tab: 0,
                name: "new".into(),
            },
            Dialog::RenameBookmark {
                uri: "/".into(),
                name: "new".into(),
            },
            Dialog::Destination {
                command: "file.copy",
                sources: vec![root.clone()],
                dest: "/".into(),
            },
            Dialog::SelectPattern {
                pattern: "*".into(),
                mode: "replace",
            },
            Dialog::Playlist {
                save: false,
                path: "/music/list.m3u8".into(),
            },
            Dialog::Playlist {
                save: true,
                path: "/music/queue.m3u8".into(),
            },
            Dialog::ConfirmDelete {
                sources: vec![root.clone()],
            },
            Dialog::ConfirmRun {
                uri: root.clone(),
                name: "program".into(),
            },
        ] {
            let id = Id::NULL.with(match &dialog {
                Dialog::ConfirmDelete { .. } => "fm-confirm-delete",
                Dialog::ConfirmRun { .. } => "fm-confirm-run",
                Dialog::SelectPattern { .. } => "fm-pattern-dialog",
                _ => "fm-name-dialog",
            });
            let mut dialogs = Dialogs::default();
            dialogs.request(dialog);
            let mut h = Harness::builder()
                .with_size(egui::vec2(1000.0, 800.0))
                .build_ui_state(
                    move |ui, (dialogs, s): &mut (Dialogs, Session)| {
                        fm_ui::theme::apply_to(ui.ctx(), &tokens);
                        dialogs.show(ui.ctx(), s, &tokens);
                    },
                    (dialogs, Session::new().expect("session")),
                );
            drag(&mut h, id);
            h.get_by_label("Close window").click();
            h.key_press(egui::Key::Enter);
            h.run();
            assert!(!h.state().0.is_open());
            assert!(h.state().1.fileops.jobs().is_empty());
            assert!(h.state().1.running);
        }
    }
}

#[test]
fn key_file_browse_uses_the_shared_picker_and_fills_the_field() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let dir = tempfile::tempdir().expect("tmp");
        let key = dir.path().join("id_test");
        std::fs::write(&key, "k").expect("key");
        let mut s = Session::new().expect("session");
        s.remote.sftp_prompt = Some(SftpPrompt {
            host: "example.test".into(),
            key_path: key.to_string_lossy().to_string(),
            ..Default::default()
        });
        let both: fn(&egui::Context, &mut Session, &Tokens) = |ctx, s, t| {
            s.poll();
            fm_ui::sftp_popup::sftp_prompt_dialog(ctx, s, t);
            fm_ui::picker::show(ctx, s, t);
        };
        let mut h = harness(tokens, s, both);
        h.get_by_label("Browse…").click();
        h.run_steps(3);
        assert_eq!(
            h.state().picker.as_ref().map(|p| p.purpose.clone()),
            Some("sftp.key".to_string())
        );
        // Escape closes the chooser, not the connection form.
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().picker.is_none());
        assert!(h.state().remote.sftp_prompt.is_some());
        h.get_by_label("Host");

        h.get_by_label("Browse…").click();
        for _ in 0..100 {
            h.run_steps(2);
            if h.query_by_label("id_test").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.get_by_label("id_test").click();
        h.run_steps(2);
        h.get_by_label("Choose file").click();
        h.run_steps(4);
        assert!(h.state().picker.is_none());
        let want = key.to_string_lossy().to_string();
        let has = h
            .query_all_by_role(egui::accesskit::Role::TextInput)
            .any(|n| n.accesskit_node().value().is_some_and(|v| v == want));
        assert!(has, "the key file field was not filled");
    }
}

#[test]
fn a_chooser_opened_from_the_sign_in_form_comes_to_the_front() {
    let tokens = Tokens::dark();
    let dir = tempfile::tempdir().expect("tmp");
    let start = dir.path().to_string_lossy().to_string();
    let mut s = Session::new().expect("session");
    s.dispatch(
        "picker.open",
        &json!({"purpose": "sync.left", "start": start}),
    )
    .expect("first");
    s.remote.sftp_prompt = Some(SftpPrompt {
        host: "example.test".into(),
        ..Default::default()
    });
    let both: fn(&egui::Context, &mut Session, &Tokens) = |ctx, s, t| {
        s.poll();
        fm_ui::picker::show(ctx, s, t);
        fm_ui::sftp_popup::sftp_prompt_dialog(ctx, s, t);
    };
    let mut h = harness(tokens, s, both);
    h.run_steps(4);
    h.state_mut()
        .dispatch(
            "picker.open",
            &json!({"purpose": "sftp.key", "kind": "file", "start": start}),
        )
        .expect("second");
    h.run_steps(4);
    let popup = h
        .ctx
        .memory(|m| m.area_rect(Id::NULL.with("fm-sftp-prompt").with("window")))
        .expect("popup");
    let top = h.ctx.memory(|m| m.layer_id_at(popup.center()));
    assert_eq!(
        top.map(|l| l.id),
        Some(Id::NULL.with("fm-picker")),
        "the chooser stayed behind the sign-in form"
    );
    h.state_mut()
        .dispatch("picker.cancel", &json!({}))
        .expect("cancel");
    h.run_steps(4);
    let top = h.ctx.memory(|m| m.layer_id_at(popup.center()));
    assert_eq!(
        top.map(|l| l.id),
        Some(Id::NULL.with("fm-sftp-prompt").with("window")),
        "the earlier chooser covered the sign-in form"
    );
}

#[test]
fn a_sign_in_form_opened_by_a_chooser_appears_in_front_of_it() {
    let tokens = Tokens::dark();
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = Session::new().expect("session");
    let both: fn(&egui::Context, &mut Session, &Tokens) = |ctx, s, t| {
        s.poll();
        fm_ui::sftp_popup::sftp_prompt_dialog(ctx, s, t);
        fm_ui::picker::show(ctx, s, t);
    };
    // The form was used before, so its layer already sits low in the stacking order.
    s.remote.sftp_prompt = Some(SftpPrompt {
        host: "example.test".into(),
        ..Default::default()
    });
    let mut h = harness(tokens, s, both);
    h.run_steps(4);
    h.state_mut().remote.sftp_prompt = None;
    h.run_steps(2);
    h.state_mut()
        .dispatch(
            "picker.open",
            &json!({"purpose": "sync.left", "start": dir.path().to_string_lossy()}),
        )
        .expect("picker");
    h.run_steps(4);
    let location = h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .expect("location");
    location.click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("sftp://me@nas.example/".into()));
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(6);
    assert!(h.state().remote.sftp_prompt.is_some(), "no sign-in form");
    let popup = h
        .ctx
        .memory(|m| m.area_rect(Id::NULL.with("fm-sftp-prompt").with("window")))
        .expect("popup");
    let top = h.ctx.memory(|m| m.layer_id_at(popup.center()));
    assert_eq!(
        top.map(|l| l.id),
        Some(Id::NULL.with("fm-sftp-prompt").with("window")),
        "the chooser covered the sign-in form"
    );
    assert_eq!(
        top.map(|l| l.order),
        Some(egui::Order::Foreground),
        "sign-in forms must live above the chooser layer"
    );
}

#[test]
fn playlist_dialog_from_media_dispatches_load_without_a_files_selection() {
    let fs = fm_testkit::MemFs::new();
    fs.mkdirs_str("/music").expect("folder");
    fs.put_str("/music/list.m3u8", b"song.mp3\n")
        .expect("playlist");
    fs.put_str("/music/song.mp3", b"song").expect("audio");
    let mut s = Session::new().expect("session");
    s.backends.register(std::sync::Arc::new(fs));
    s.layout.state_mut(s.layout.focused()).expect("pane").kind = fm_layout::PaneKind::Media;
    let mut dialog =
        fm_ui::dialogs::dialog_for("media.playlist.load", &s).expect("dialog from Media");
    if let Dialog::Playlist { path, .. } = &mut dialog {
        *path = "/music/list.m3u8".into();
    }
    let mut dialogs = Dialogs::default();
    dialogs.request(dialog);
    let mut h = Harness::builder().build_ui_state(
        |ui, (dialogs, s): &mut (Dialogs, Session)| {
            dialogs.show(ui.ctx(), s, &Tokens::dark());
        },
        (dialogs, s),
    );
    h.run();
    h.get_by_label("OK").click();
    h.run();
    assert!(!h.state().0.is_open());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while h.state().1.media.queue.items().is_empty() {
        h.state_mut().1.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "playlist load timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        h.state().1.media.queue.items()[0].file_name(),
        Some("song.mp3")
    );
}

type Show = fn(&Context, &mut Session, &Tokens);

#[test]
fn manual_smb_form_connects_with_a_unc_server_name() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.dispatch("network.connect.dialog", &json!(null))
            .expect("dialog");
        let mut h = harness(tokens, s, fm_ui::remote::login_dialog);
        for (label, text) in [
            ("Host", r"\\127.0.0.1"),
            ("Port", "1"),
            ("Username", "alice"),
        ] {
            h.get_by_label(label).click();
            h.run();
            h.get_by_label(label).type_text(text);
            h.run();
        }
        h.get_by_label("Connect").click();
        h.run();
        assert!(h.state().remote.login_prompt.is_none());
        let server = VfsUri::parse("smb://127.0.0.1:1/").expect("server");
        assert_eq!(
            h.state()
                .remote
                .state(fm_vfs::Scheme::Smb, server.authority()),
            Some(&fm_engine::remote::ConnState::Connecting)
        );
    }
}

#[test]
fn properties_title_close_cancels_the_background_count() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let fs = fm_testkit::MemFs::new();
        let mut s = Session::new().expect("session");
        s.backends.register(std::sync::Arc::new(fs));
        s.dispatch("file.properties", &json!({"sources": ["/"]}))
            .expect("properties");
        let mut h = harness(tokens, s, fm_ui::properties::show);
        drag(&mut h, Id::NULL.with("fm-properties"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().properties.is_none());
    }
}

#[test]
#[cfg(not(windows))]
fn open_with_title_close_does_not_launch_an_application() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.dispatch("file.open_with.dialog", &json!({"uri": "/example.txt"}))
            .expect("open with");
        let mut h = harness(tokens, s, fm_ui::open_with::show);
        drag(&mut h, Id::NULL.with("fm-open-with"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().open_with.is_none());
    }
}

fn harness(tokens: Tokens, session: Session, show: Show) -> Harness<'static, Session> {
    Harness::builder()
        .with_size(egui::vec2(1100.0, 850.0))
        .build_ui_state(
            move |ui, s| {
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                show(ui.ctx(), s, &tokens);
            },
            session,
        )
}

#[test]
fn preferences_connections_alerts_and_search_close_from_title_bar() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.preferences_open = true;
        let mut h = harness(tokens, s, fm_ui::preferences::show);
        drag(&mut h, Id::NULL.with("fm-preferences"));
        let before = h.state().prefs.check_updates;
        h.get_by_label("Check for updates at startup").click();
        h.run();
        h.get_by_label("Close window").click();
        h.run();
        assert!(!h.state().preferences_open);
        assert_eq!(h.state().prefs.check_updates, !before);

        let mut s = Session::new().expect("session");
        s.remote.sftp_prompt = Some(SftpPrompt {
            host: "example.test".into(),
            ..Default::default()
        });
        let mut h = harness(tokens, s, fm_ui::sftp_popup::sftp_prompt_dialog);
        drag(&mut h, Id::NULL.with("fm-sftp-prompt").with("window"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().remote.sftp_prompt.is_none());
        assert!(!h.state().remote.is_connecting());

        let mut s = Session::new().expect("session");
        s.dispatch("network.connect.dialog", &json!({}))
            .expect("SMB form");
        let mut h = harness(tokens, s, fm_ui::remote::login_dialog);
        drag(&mut h, Id::NULL.with("fm-login-dialog").with("window"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().remote.login_prompt.is_none());

        let mut s = Session::new().expect("session");
        s.remote.host_key_alert = Some("The host key changed.".into());
        let mut h = harness(tokens, s, fm_ui::remote::host_key_alert);
        drag(&mut h, Id::NULL.with("fm-hostkey-alert"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().remote.host_key_alert.is_none());

        let mut s = Session::new().expect("session");
        s.dispatch("search.open", &json!({"root": "/"}))
            .expect("search");
        let mut h = harness(tokens, s, fm_ui::search::show);
        drag(&mut h, Id::NULL.with("fm-search"));
        h.get_by_label("Minimize window").click();
        h.run();
        assert!(h.state().search.as_ref().expect("search").minimized);
        h.state_mut()
            .dispatch("search.restore", &json!(null))
            .expect("restore");
        h.run();
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().search.is_none());
    }
}

#[test]
fn closing_unsaved_prompt_does_not_quit_or_discard() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.unsaved_prompt = Some(fm_engine::workspace::UnsavedPrompt {
            after: fm_engine::workspace::AfterPrompt::Quit,
            names: vec!["Files".into()],
        });
        let mut dialogs = Dialogs::default();
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(move |ui, s| dialogs.show(ui.ctx(), s, &tokens), s);
        drag(&mut h, Id::NULL.with("fm-unsaved-workspaces"));
        h.get_by_label("Close window").click();
        h.run();
        assert!(h.state().unsaved_prompt.is_none());
        assert!(h.state().running);
    }
}

#[test]
fn closing_jobs_hides_without_dismissing_update_and_indicator_reopens() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.updates.visible = true;
        s.updates.status = fm_engine::updates::UpdateStatus::Available {
            version: "0.2.0".into(),
            url: String::new(),
        };
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_ui_state(
                move |ui, s| {
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::jobs::indicator(ui, s, &tokens);
                    fm_ui::jobs::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        h.get_by_label("Close window").click();
        h.run_steps(3);
        assert!(!fm_ui::jobs::panel_visible(&h.ctx, h.state()));
        assert!(h.state().updates.visible);
        h.get_by_label("Update available").click();
        h.run();
        assert!(fm_ui::jobs::panel_visible(&h.ctx, h.state()));
        h.get_by_label("Close window");
    }
}

#[test]
fn connection_forms_keep_title_close_and_cancel_visible_in_small_windows() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        for smb in [false, true] {
            let mut s = Session::new().expect("session");
            let show: Show = if smb {
                s.dispatch("network.connect.dialog", &json!({}))
                    .expect("SMB form");
                fm_ui::remote::login_dialog
            } else {
                s.remote.sftp_prompt = Some(SftpPrompt::default());
                fm_ui::sftp_popup::sftp_prompt_dialog
            };
            let mut h = harness(tokens, s, show);
            h.set_size(egui::vec2(560.0, 320.0));
            h.run_steps(5);
            for label in ["Cancel", "Close window"] {
                assert!(
                    h.ctx
                        .content_rect()
                        .contains_rect(h.get_by_label(label).rect()),
                    "{label}"
                );
            }
        }
    }
}

#[test]
fn closing_conflict_cancels_the_copy_without_overwriting() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let fs = fm_testkit::MemFs::new();
        fs.put_str("/src/a.txt", b"new").expect("source");
        fs.put_str("/dst/a.txt", b"old").expect("destination");
        let mut s = Session::new().expect("session");
        s.backends.register(std::sync::Arc::new(fs.clone()));
        s.dispatch(
            "file.copy",
            &json!({"sources": ["/src/a.txt"], "dest": "/dst"}),
        )
        .expect("copy");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while s.fileops.pending_conflict().is_none() {
            s.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let mut dialogs = Dialogs::default();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 800.0))
            .build_ui_state(move |ui, s| dialogs.show(ui.ctx(), s, &tokens), s);
        drag(&mut h, Id::NULL.with("fm-conflict-dialog"));
        h.get_by_label("Close window").click();
        h.run();
        while h
            .state()
            .fileops
            .jobs()
            .iter()
            .any(|j| j.state.is_running())
        {
            h.state_mut().poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(matches!(
            h.state().fileops.jobs()[0].state,
            fm_engine::OpState::Cancelled
        ));
        assert_eq!(fs.get_str("/dst/a.txt").expect("unchanged"), b"old");
    }
}

#[test]
fn compare_with_lists_the_folder_picks_a_file_and_cancels() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::write(dir.path().join("a.txt"), "1\n").expect("write");
        std::fs::write(dir.path().join("b.txt"), "2\n").expect("write");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let a = VfsUri::parse(&dir.path().join("a.txt").to_string_lossy()).expect("uri");
        let b = VfsUri::parse(&dir.path().join("b.txt").to_string_lossy()).expect("uri");
        let mut s = Session::new().expect("session");
        s.dispatch("file.compare_with", &json!({"uri": a.to_string()}))
            .expect("compare with");
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    s.poll();
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::compare_with::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        // While the folder loads, a spinner keeps repainting: step instead of `run`, which
        // fails when the UI does not settle within a few frames (slow CI runners).
        for _ in 0..200 {
            h.run_steps(2);
            if h.query_by_label("b.txt").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run();
        assert!(h.query_by_label("sub/").is_some());
        assert!(
            h.query_by_label("a.txt").is_none(),
            "the source is not offered"
        );
        h.get_by_label("b.txt").click();
        h.run();
        assert_eq!(
            h.state().compare.as_ref().and_then(|c| c.selected.clone()),
            Some(b.clone())
        );
        h.get_by_label("Cancel").click();
        h.run();
        assert!(h.state().compare.is_none());
    }
}

#[test]
fn picker_opens_a_folder_enters_it_and_sets_the_sync_side() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let dir = tempfile::tempdir().expect("tmp");
        std::fs::create_dir(dir.path().join("photos")).expect("mkdir");
        std::fs::write(dir.path().join("note.txt"), "x").expect("write");
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        s.dispatch(
            "picker.open",
            &json!({"purpose": "sync.left", "kind": "folder",
                    "start": dir.path().to_string_lossy()}),
        )
        .expect("picker");
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    s.poll();
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::picker::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        for _ in 0..200 {
            h.run_steps(2);
            if h.query_by_label("photos").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run();
        assert!(
            h.query_by_label("note.txt").is_none(),
            "a folder picker lists folders only"
        );
        h.get_by_label("photos").click();
        h.run();
        eprintln!(
            "DBG sel={:?}",
            h.state().picker.as_ref().map(|p| p.selected.clone())
        );
        h.get_by_label("Choose folder").click();
        h.run();
        assert!(
            h.state().picker.is_none(),
            "{:?} {:?}",
            h.state().notice,
            h.state().picker
        );
        let want = VfsUri::parse(&dir.path().join("photos").to_string_lossy()).expect("uri");
        let left = h
            .state()
            .sync
            .current_state()
            .map(|st| st.profile.left.clone());
        assert_eq!(left, Some(want.to_string()));
    }
}

#[test]
fn sync_window_width_is_stable_across_frames() {
    for view in ["wizard", "advanced"] {
        let tokens = Tokens::dark();
        let mut s = Session::new().expect("session");
        s.dispatch("sync.open", &json!({})).expect("open");
        s.dispatch("sync.view", &json!({"mode": view}))
            .expect("view");
        let mut h = Harness::builder()
            .with_size(egui::vec2(1200.0, 800.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::sync::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        let width = |h: &Harness<'_, Session>| {
            h.ctx
                .memory(|m| m.area_rect(Id::NULL.with("fm-sync")))
                .map(|r| r.width())
                .unwrap_or_default()
        };
        h.run_steps(5);
        let first = width(&h);
        h.run_steps(30);
        let last = width(&h);
        assert!(first > 0.0, "{view}: window missing");
        assert!((last - first).abs() < 1.0, "{view}: {first} grew to {last}");
    }
}

#[test]
fn picker_size_is_stable_and_places_navigate() {
    let tokens = Tokens::dark();
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "picker.open",
        &json!({"purpose": "x", "start": dir.path().to_string_lossy()}),
    )
    .expect("picker");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                s.poll();
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::picker::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    let size = |h: &Harness<'_, Session>| {
        h.ctx
            .memory(|m| m.area_rect(Id::NULL.with("fm-picker")))
            .map(|r| r.size())
            .unwrap_or_default()
    };
    h.run_steps(5);
    let first = size(&h);
    h.get_by_label("Saved servers").scroll_to_me();
    h.run_steps(3);
    h.get_by_label("Saved servers").click();
    h.run_steps(30);
    let last = size(&h);
    assert!(first.x > 0.0);
    assert!((last - first).length() < 1.0, "{first:?} -> {last:?}");
    let folder = h.state().picker.as_ref().map(|p| p.folder.to_string());
    assert_eq!(folder.as_deref(), Some("servers:///"));
}

#[test]
fn picker_new_folder_creates_it_and_escape_closes_the_row_first() {
    let tokens = Tokens::dark();
    let dir = tempfile::tempdir().expect("tmp");
    let mut s = Session::new().expect("session");
    s.dispatch(
        "picker.open",
        &json!({"purpose": "x", "start": dir.path().to_string_lossy()}),
    )
    .expect("picker");
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                s.poll();
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::picker::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    h.run_steps(5);
    h.get_by_label("New folder").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.state().picker.is_some(), "Esc closed the whole picker");
    h.get_by_label("New folder").click();
    h.run_steps(3);
    h.event(egui::Event::Text("fresh".into()));
    h.run_steps(2);
    h.get_by_label("Create").click();
    for _ in 0..200 {
        h.run_steps(2);
        if dir.path().join("fresh").is_dir() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(dir.path().join("fresh").is_dir());
}

#[test]
fn a_minimised_search_shows_in_the_jobs_panel_and_comes_back() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(tmp.path().join("report.pdf"), b"x").expect("file");
    let root = tmp.path().to_str().expect("utf8").to_string();
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.dispatch("search.start", &json!({"root": root, "name": "*.pdf"}))
            .expect("start");
        s.dispatch("search.minimize", &json!(null))
            .expect("minimize");
        let mut h = Harness::builder()
            .with_size(egui::vec2(1000.0, 700.0))
            .build_ui_state(
                move |ui, s| {
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::jobs::icon_button(ui, s, &tokens);
                    fm_ui::jobs::show(ui.ctx(), s, &tokens);
                    fm_ui::search::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        // Wait for the walk to finish, then open the panel as the user would.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while h
            .state()
            .search
            .as_ref()
            .is_some_and(|x| x.status().running)
        {
            assert!(std::time::Instant::now() < deadline);
            h.run_steps(2);
        }
        h.run();
        if !fm_ui::jobs::panel_visible(&h.ctx, h.state()) {
            h.get_by_label("Activity").click();
            h.run();
        }
        h.get_by_label_contains("Find \"*.pdf\"");
        h.get_by_label_contains("1 match");
        assert!(
            h.query_by_label("Find Files").is_none(),
            "window hidden while minimised"
        );
        h.get_by_label("Show").click();
        h.run();
        assert!(!h.state().search.as_ref().expect("search").minimized);
        h.get_by_label("Find Files");
    }
}

#[test]
fn right_clicking_a_found_file_opens_its_menu() {
    let tmp = tempfile::tempdir().expect("tmp");
    std::fs::write(tmp.path().join("report.pdf"), b"x").expect("file");
    let root = tmp.path().to_str().expect("utf8").to_string();
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.dispatch("search.start", &json!({"root": root, "name": "*.pdf"}))
            .expect("start");
        let mut h = harness(tokens, s, fm_ui::search::show);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while h
            .state()
            .search
            .as_ref()
            .is_some_and(|x| x.status().running)
        {
            assert!(std::time::Instant::now() < deadline);
            h.run_steps(2);
        }
        h.run();
        let pos = h.get_by_label("report.pdf").rect().center();
        for pressed in [true, false] {
            h.input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.step();
        }
        h.run();
        h.get_by_label("Open With…");
        h.get_by_label("Properties");
        h.get_by_label("Copy Name").click();
        h.run();
        assert!(h.query_by_label("Open With…").is_none(), "menu closes");
    }
}

#[test]
fn renaming_a_workspace_replaces_the_old_name_as_you_type() {
    let tokens = Tokens::dark();
    let s = Session::new().expect("session");
    let old = s
        .workspaces
        .tabs
        .first()
        .map(|t| t.name.clone())
        .expect("tab");
    let mut dialogs = Dialogs::default();
    dialogs.request(Dialog::RenameWorkspace {
        tab: 0,
        name: old.clone(),
    });
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 800.0))
        .build_ui_state(
            move |ui, (dialogs, s): &mut (Dialogs, Session)| {
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                dialogs.show(ui.ctx(), s, &tokens);
            },
            (dialogs, s),
        );
    h.run();
    h.event(egui::Event::Text("Music".into()));
    h.run();
    h.key_press(egui::Key::Enter);
    h.run();
    assert!(!h.state().0.is_open());
    let name = h.state().1.workspaces.tabs.first().map(|t| t.name.clone());
    assert_eq!(name.as_deref(), Some("Music"), "old name was {old:?}");
}

fn sync_session(files: &[(&str, &str, u64)], mode: &str) -> Session {
    let fs = fm_testkit::MemFs::new();
    fs.mkdirs_str("/s/a").expect("mkdirs");
    fs.mkdirs_str("/s/b").expect("mkdirs");
    for (dir, name, secs) in files {
        let uri = VfsUri::parse(&format!("/s/{dir}/{name}")).expect("uri");
        let mtime = std::time::UNIX_EPOCH + std::time::Duration::from_secs(*secs);
        fs.put_file_at(&uri, name.as_bytes(), mtime).expect("put");
    }
    let mut s = Session::new().expect("session");
    s.backends.register(std::sync::Arc::new(fs));
    s.dispatch("sync.open", &json!({})).expect("open");
    s.dispatch("sync.view", &json!({"mode": "advanced"}))
        .expect("advanced");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"left": "/s/a", "right": "/s/b", "mode": mode}}),
    )
    .expect("set");
    s.dispatch("sync.compare", &json!({})).expect("compare");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while s
        .sync
        .current_state()
        .is_none_or(|st| st.busy() || st.plan.is_none())
    {
        s.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "compare never finished"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    s
}

#[test]
fn sync_wizard_walks_through_the_questions_and_starts_the_sync() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = sync_session(
            &[("a", "x.txt", 2_000_000), ("b", "y.txt", 1_000_000)],
            "Update",
        );
        s.dispatch("sync.wizard.go", &json!({"step": 0}))
            .expect("first step");
        let mut h = harness(tokens, s, fm_ui::sync::show);
        h.run();
        h.get_by_label("Step 1 of 7: Folders");
        h.get_by_label("Next").click();
        h.run();
        h.get_by_label("Step 2 of 7: Goal");
        h.get_by_label("Merge both ways").click();
        h.run();
        let mode = h.state().sync.current_state().map(|st| st.profile.mode);
        assert_eq!(mode, Some(fm_sync::SyncMode::TwoWay));
        for _ in 0..4 {
            h.get_by_label("Next").click();
            h.run();
        }
        h.get_by_label("Step 6 of 7: Keep in sync");
        h.get_by_label("Next: compare and review").click();
        for _ in 0..200 {
            h.state_mut().poll();
            h.run();
            if h.state()
                .sync
                .current_state()
                .is_some_and(|st| !st.busy() && st.plan.is_some())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.get_by_label("Step 7 of 7: Review");
        h.get_by_label("Start: synchronize now").click();
        h.run_steps(3);
        for _ in 0..200 {
            h.state_mut().poll();
            h.run();
            if h.state()
                .sync
                .current_state()
                .is_some_and(|st| !st.busy() && st.report.is_some())
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let report = h
            .state()
            .sync
            .current_state()
            .and_then(|st| st.report.as_ref());
        assert!(report.is_some(), "the sync ran");
    }
}

#[test]
fn sync_window_changes_a_rows_action_through_its_menu() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let s = sync_session(
            &[("a", "x.txt", 2_000_000), ("b", "x.txt", 1_000_000)],
            "Update",
        );
        let mut h = harness(tokens, s, fm_ui::sync::show);
        h.run();
        h.get_by_label("x.txt: Copy to the right").click();
        h.run();
        h.get_by_label("Skip this item").click();
        h.run();
        let plan = h
            .state()
            .sync
            .current_state()
            .and_then(|st| st.plan.as_ref())
            .expect("plan");
        assert_eq!(plan.items[0].action, fm_sync::Action::Skip);
        h.get_by_label("x.txt: Skipped");
    }
}

#[test]
fn sync_grid_lays_out_only_visible_rows_of_a_huge_plan() {
    let names: Vec<String> = (0..100_000).map(|n| format!("f{n:06}.txt")).collect();
    let files: Vec<(&str, &str, u64)> = names.iter().map(|n| ("a", n.as_str(), 1)).collect();
    let s = sync_session(&files, "Update");
    let rows = s.sync.current_state().map_or(0, |st| st.rows.len());
    assert_eq!(rows, 100_000);
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    let laid_out = h
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.contains(": Copy to the right"))
        })
        .count();
    assert!(laid_out > 0 && laid_out < 200, "{laid_out} rows laid out");
}

#[test]
fn sync_permanent_deletes_ask_in_their_own_dialog() {
    let mut s = sync_session(&[("b", "extra.txt", 1_000_000)], "Mirror");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"delete": "Permanent"}}),
    )
    .expect("permanent");
    s.dispatch("sync.compare", &json!({})).expect("compare");
    for _ in 0..1000 {
        s.poll();
        if s.sync.current_state().is_some_and(|st| !st.busy()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Refused until confirmed: the state asks instead.
    let _ = s.dispatch("sync.run", &json!({}));
    assert!(s.sync.current_state().is_some_and(|st| st.confirm));
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    h.get_by_label("Delete permanently?");
    h.get_by_label("Cancel").click();
    h.run();
    assert!(h.state().sync.current_state().is_some_and(|st| !st.confirm));
    assert!(h.query_by_label("Delete permanently?").is_none());
}

#[test]
fn advanced_offers_synchronize_now_after_keep_in_sync_was_stopped() {
    let mut s = sync_session(&[("a", "x.txt", 2_000_000)], "Update");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"watch": {"enabled": true, "poll_secs": 30}}}),
    )
    .expect("keep on");
    // Stopping leaves the profile's preference set, but the checkbox and the button follow
    // whether it is actually running.
    s.dispatch("sync.watch.stop", &json!({})).expect("stop");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    h.get_by_label("Synchronize now");
    assert!(h.query_by_label("Start syncing").is_none());
}

#[test]
fn sync_grid_headers_sort_and_the_window_minimizes_to_the_status_bar() {
    let s = sync_session(
        &[("a", "small.txt", 1), ("a", "much-bigger-name.txt", 1)],
        "Update",
    );
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    h.get_all_by_label("Sort by Size")
        .next()
        .expect("header")
        .click();
    h.run();
    let st = h.state().sync.current_state().expect("state");
    assert_eq!(st.sort, fm_engine::sync_hub::SortKey::Size);
    assert!(!st.descending);

    h.state_mut()
        .dispatch("sync.watch.start", &json!({}))
        .expect("watch");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while h
        .state()
        .sync
        .current_state()
        .is_some_and(|st| st.arming != fm_engine::sync_hub::Arming::Confirm)
    {
        h.state_mut().poll();
        assert!(std::time::Instant::now() < deadline, "compare timed out");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    h.run();
    h.state_mut()
        .dispatch("sync.watch.confirm", &json!({}))
        .expect("confirm");
    h.get_by_label("Minimize to the Activity panel").click();
    h.run();
    assert!(h.state().sync.minimized);
    assert!(h.query_by_label("Synchronize Folders").is_none());
    assert!(fm_ui::jobs::indicator_text(h.state()).contains("New sync"));
}

#[test]
fn wizard_leave_out_step_adds_and_removes_patterns_in_the_list_box() {
    use egui_kittest::kittest::Queryable;
    let tokens = Tokens::dark();
    let mut s = Session::new().expect("session");
    s.dispatch("sync.open", &json!({})).expect("open");
    s.dispatch("sync.view", &json!({"mode": "wizard"}))
        .expect("view");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"left": "/tmp/a", "right": "/tmp/b"}}),
    )
    .expect("folders");
    s.dispatch("sync.wizard.go", &json!({"step": 4}))
        .expect("step");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::sync::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    h.run_steps(3);
    let exclude = |h: &Harness<'_, Session>| {
        h.state()
            .sync
            .current_state()
            .map(|st| st.profile.filters.exclude.clone())
            .unwrap_or_default()
    };
    h.get_by_label("Add pattern").click();
    h.run_steps(3);
    let input = h.get_by_role(egui::accesskit::Role::TextInput);
    input.type_text("*.tmp");
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert_eq!(exclude(&h), vec!["*.tmp".to_string()]);
    h.get_by_label("*.tmp").click();
    h.run_steps(3);
    h.get_by_label("Remove selected pattern").click();
    h.run_steps(4);
    assert!(exclude(&h).is_empty());
}

#[test]
fn wizard_back_button_stays_in_place_on_every_step() {
    let dir = tempfile::tempdir().expect("dir");
    std::fs::create_dir_all(dir.path().join("a")).expect("a");
    std::fs::create_dir_all(dir.path().join("b")).expect("b");
    std::fs::write(dir.path().join("a/x.txt"), "x").expect("file");
    use egui_kittest::kittest::Queryable;
    let tokens = Tokens::dark();
    let mut s = Session::new().expect("session");
    s.dispatch("sync.open", &json!({})).expect("open");
    s.dispatch("sync.view", &json!({"mode": "wizard"}))
        .expect("view");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"left": dir.path().join("a").to_str().unwrap_or(""), "right": dir.path().join("b").to_str().unwrap_or("")}}),
    )
    .expect("folders");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                s.poll();
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::sync::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    let mut spots = Vec::new();
    for step in 0..=6 {
        if step == 6 {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(25));
                h.run_steps(2);
            }
        }
        h.state_mut()
            .dispatch("sync.wizard.go", &json!({"step": step}))
            .expect("step");
        h.run_steps(4);
        spots.push(h.get_by_label("Back").rect().min);
    }
    let start = h.get_by_label("Start: synchronize now").rect();
    let back = h.get_by_label("Back").rect();
    assert!(
        start.right() > 1100.0 - 80.0 && start.left() - back.right() > 300.0,
        "Start belongs in the far corner: {start:?} vs {back:?}"
    );
    assert!(
        spots.windows(2).all(|w| (w[0] - w[1]).length() < 0.5),
        "{spots:?}"
    );
}

#[test]
fn wizard_first_step_loads_a_saved_sync() {
    use egui_kittest::kittest::Queryable;
    let tokens = Tokens::dark();
    let mut s = Session::new().expect("session");
    s.dispatch("sync.open", &json!({})).expect("open");
    s.dispatch("sync.view", &json!({"mode": "wizard"}))
        .expect("view");
    s.dispatch(
        "sync.profile.set",
        &json!({"patch": {"name": "Holiday", "left": "/tmp/a", "right": "/tmp/b"}}),
    )
    .expect("set");
    s.dispatch("sync.profile.save", &json!({})).expect("save");
    s.dispatch("sync.profile.new", &json!({})).expect("new");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                s.poll();
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::sync::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    h.run_steps(3);
    h.get_by_label("Open saved").click();
    h.run_steps(3);
    h.get_by_label("Holiday").click();
    h.run_steps(4);
    let left = h
        .state()
        .sync
        .current_state()
        .map(|st| st.profile.left.clone())
        .unwrap_or_default();
    assert!(left.contains("/tmp/a"), "{left}");
}

#[test]
fn sync_inputs_follow_a_folder_chosen_in_the_picker() {
    let dir = tempfile::tempdir().expect("tmp");
    let s = sync_session(&[("a", "x.txt", 1)], "Update");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    let want = dir.path().to_string_lossy().to_string();
    let st = h.state_mut();
    st.dispatch(
        "picker.open",
        &json!({"purpose": "sync.left", "kind": "folder", "start": want}),
    )
    .expect("open");
    st.dispatch("picker.accept", &json!({})).expect("accept");
    h.run();
    let has = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .any(|n| {
            n.accesskit_node()
                .value()
                .is_some_and(|v| v.contains(&want))
        });
    assert!(has, "the input kept the old folder");
}

#[test]
fn escape_closes_the_sync_window() {
    let s = sync_session(&[("a", "x.txt", 1)], "Update");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    assert!(h.state().sync.current_state().is_some());
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.state().sync.current_state().is_none());
}

#[test]
fn sync_tree_folds_a_folder_and_shows_full_paths_for_the_selection() {
    let s = sync_session(
        &[("a", "docs/one.txt", 1), ("a", "docs/two.txt", 1)],
        "Update",
    );
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    let markers = |h: &Harness<'static, Session>| {
        h.query_all_by_role(egui::accesskit::Role::Button)
            .filter(|n| {
                n.accesskit_node()
                    .label()
                    .is_some_and(|l| l.starts_with("docs/one.txt:"))
            })
            .count()
    };
    assert_eq!(markers(&h), 1);
    h.get_by_label("Collapse folder docs").click();
    h.run();
    assert!(
        h.query_by_label("Expand folder docs").is_some(),
        "not toggled"
    );
    assert_eq!(markers(&h), 0);
    h.get_by_label("Expand folder docs").click();
    h.run();
    assert_eq!(markers(&h), 1);
}

#[test]
fn sync_settings_fold_after_compare_and_the_table_expands_into_its_own_window() {
    let s = sync_session(&[("a", "x.txt", 1)], "Update");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    assert!(h.query_by_label("Change settings").is_some());
    assert!(h.query_by_label("Deleting").is_none());
    h.get_by_label("Change settings").click();
    h.run();
    assert!(h.query_by_label("Deleting").is_some());

    h.get_by_label("Expand table").click();
    h.run();
    assert!(h.query_by_label("All changes").is_some());
    // Escape closes the large window first and leaves the sync window open.
    h.key_press(egui::Key::Escape);
    h.run();
    assert!(h.query_by_label("All changes").is_none());
    assert!(h.state().sync.current_state().is_some());
}

#[test]
fn a_rows_action_menu_adds_it_to_the_ignore_list() {
    let s = sync_session(&[("a", "x.txt", 1)], "Update");
    let mut h = harness(Tokens::dark(), s, |ctx, s, tokens| {
        s.poll();
        fm_ui::sync::show(ctx, s, tokens);
    });
    h.run();
    h.get_by_label("x.txt: Copy to the right").click();
    h.run();
    h.get_by_label("Add to ignore list").click();
    h.run();
    let state = h.state().sync.current_state().expect("sync");
    assert_eq!(state.profile.filters.exclude, ["x.txt"]);
}

#[test]
fn the_action_header_filters_rows_by_action() {
    let s = sync_session(&[("a", "x.txt", 1)], "Update");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    h.get_by_label("Filter by action").click();
    h.run();
    h.get_by_label("Skipped").click();
    h.run();
    let state = h.state().sync.current_state().expect("sync");
    assert_eq!(state.action_filter, Some(fm_sync::Action::Skip));
    assert!(state.rows.is_empty());
}

#[test]
fn sync_large_window_fills_its_table_with_rows() {
    let names: Vec<String> = (0..200).map(|n| format!("f{n:03}.txt")).collect();
    let files: Vec<(&str, &str, u64)> = names.iter().map(|n| ("a", n.as_str(), 1)).collect();
    let s = sync_session(&files, "Update");
    let mut h = harness(Tokens::dark(), s, fm_ui::sync::show);
    h.run();
    h.get_by_label("Expand table").click();
    h.run();
    h.run_steps(5);
    let rows = h
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.contains(": Copy to the right"))
        })
        .count();
    let max_y = h
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter(|n| {
            n.accesskit_node()
                .label()
                .is_some_and(|l| l.contains(": Copy to the right"))
        })
        .filter_map(|n| n.accesskit_node().bounding_box().map(|b| b.y1))
        .fold(0.0_f64, f64::max);
    assert!(rows >= 20 && max_y > 750.0, "{rows} rows, last at {max_y}");
}
