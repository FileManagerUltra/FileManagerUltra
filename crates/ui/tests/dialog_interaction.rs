#![allow(clippy::expect_used)]

use egui::{Context, Id};
use egui_kittest::{Harness, kittest::Queryable};
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
fn closing_key_picker_leaves_connection_form_open() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let home = fm_engine::workspace::home_location();
        let ssh = home.join(".ssh").expect("SSH directory");
        let fs = fm_testkit::MemFs::new();
        fs.mkdirs(&ssh).expect("fixture");
        let mut s = Session::new().expect("session");
        s.backends.register(std::sync::Arc::new(fs));
        s.remote.sftp_prompt = Some(SftpPrompt {
            host: "example.test".into(),
            key_path: "id_ed25519".into(),
            ..Default::default()
        });
        s.submit_read(ssh.clone()).expect("read");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !matches!(s.dirs.state(&ssh), fm_engine::DirState::Ready { .. }) {
            s.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        let mut h = harness(tokens, s, fm_ui::sftp_popup::sftp_prompt_dialog);
        h.get_by_label("Browse…").click();
        h.run_steps(3);
        let picker = h
            .ctx
            .memory(|m| m.area_rect(Id::NULL.with("fm-key-picker")))
            .expect("picker");
        let close = h
            .get_all_by_label("Close window")
            .find(|button| picker.contains(button.rect().center()))
            .expect("picker close");
        close.click();
        h.run_steps(3);
        assert!(h.state().remote.sftp_prompt.is_some());
        assert_eq!(h.get_all_by_label("Close window").count(), 1);
        h.get_by_label("Host");
    }
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
        for _ in 0..40 {
            h.run();
            if h.query_by_label("b.txt").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
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
            h.get_by_label("Jobs").click();
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
