//! Media open preference, Activity hint, playlist reveal and the player menu
//! (change `media-open-and-play-ux`), driven through the real widgets.
#![allow(clippy::expect_used)]

use egui_kittest::{Harness, kittest::Queryable};
use fm_engine::Session;
use fm_engine::messages::MessageAction;
use fm_engine::prefs::MediaOpenTarget;
use fm_ui::Tokens;
use serde_json::json;

fn media_harness(tokens: Tokens, mut s: Session, uris: &[&str]) -> Harness<'static, Session> {
    s.dispatch("media.enqueue", &json!({ "uris": uris }))
        .expect("enqueue");
    let pane = s.media_pane().expect("media pane");
    Harness::builder()
        .with_size(egui::vec2(640.0, 480.0))
        .build_ui_state(
            move |ui, s| {
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                egui::CentralPanel::default().show(ui, |ui| {
                    fm_ui::media::show(ui, s, pane, &tokens);
                });
            },
            s,
        )
}

/// Right-click empty player space (left of the title, where the cover sits).
fn right_click_player(h: &mut Harness<'_, Session>) {
    let rect = h
        .ctx
        .data(|d| d.get_temp::<egui::Rect>(fm_ui::media::player_rect_id()))
        .expect("player area");
    let pos = rect.left_top() + egui::vec2(30.0, 30.0);
    h.hover_at(pos);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    h.run();
}

#[test]
fn the_hint_buttons_in_the_activity_panel_act_once() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let mut s = Session::new().expect("session");
        s.messages.push_hint(
            "You can also play audio and video here.",
            "media-open",
            vec![MessageAction {
                label: "Always play here".into(),
                commands: vec![(
                    "prefs.media_open".into(),
                    json!({"kind": "audio", "target": "media"}),
                )],
            }],
        );
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui_state(
                move |ui, s| {
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    fm_ui::jobs::show(ui.ctx(), s, &tokens);
                },
                s,
            );
        // The first frame records the log; the hint opens the panel on the next push.
        h.run();
        h.state_mut().messages.push("unrelated");
        h.run();
        h.get_by_label("Activity");
        h.get_by_label("Always play here").click();
        h.run();
        assert_eq!(h.state().prefs.media_open_audio, MediaOpenTarget::MediaPane);
        assert!(
            h.query_by_label("Always play here").is_none(),
            "a used hint shows no buttons"
        );
    }
}

#[test]
fn right_clicking_the_audio_player_opens_its_menu_without_video_entries() {
    for tokens in [Tokens::dark(), Tokens::light()] {
        let s = Session::new().expect("session");
        let mut h = media_harness(tokens, s, &["/music/song.mp3"]);
        h.run();
        right_click_player(&mut h);
        h.get_by_label("Equalizer…");
        h.get_by_label("Show in folder");
        assert!(
            h.query_by_label("Fullscreen").is_none(),
            "audio has no fullscreen"
        );
        assert!(h.query_by_label("Subtitle appearance…").is_none());
        assert!(
            !h.state().media.playing,
            "a right-click never toggles playback"
        );
        h.get_by_label("Show in folder").click();
        h.run();
        assert!(
            h.query_by_label("Equalizer…").is_none(),
            "choosing an entry closes the menu"
        );
    }
}

#[test]
fn added_rows_are_selected_once_and_scrolled_into_view() {
    let s = Session::new().expect("session");
    let first: Vec<String> = (0..300).map(|n| format!("/music/{n:03}.mp3")).collect();
    let first: Vec<&str> = first.iter().map(String::as_str).collect();
    let mut h = media_harness(Tokens::dark(), s, &first);
    h.run();
    let pane = h.state().media_pane().expect("pane");
    let scrolled = |h: &Harness<'_, Session>| {
        h.state()
            .layout
            .state(pane)
            .map(|p| p.media.queue_scroll)
            .unwrap_or_default()
    };
    // The first add selected rows 0..300 and starts at the top.
    assert_eq!(scrolled(&h), 0.0);
    h.state_mut()
        .dispatch(
            "media.enqueue",
            &json!({"uris": ["/music/a.mp3", "/music/b.mp3", "/music/c.mp3"]}),
        )
        .expect("add");
    h.run_steps(60);
    let picked: Vec<usize> = h
        .state()
        .layout
        .state(pane)
        .map(|p| p.media.picked.iter().copied().collect())
        .unwrap_or_default();
    assert_eq!(picked, [300, 301, 302]);
    assert!(scrolled(&h) > 1000.0, "row 301 is brought into view");
    // Seen once: the user can select something else afterwards.
    h.get_by_label("a").click();
    h.run_steps(10);
    let picked: Vec<usize> = h
        .state()
        .layout
        .state(pane)
        .map(|p| p.media.picked.iter().copied().collect())
        .unwrap_or_default();
    assert_eq!(picked, [300]);
}

#[test]
fn preferences_offer_the_open_choice_per_kind() {
    let mut s = Session::new().expect("session");
    s.preferences_open = true;
    let tokens = Tokens::dark();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1100.0, 850.0))
        .build_ui_state(
            move |ui, s| {
                fm_ui::theme::apply_to(ui.ctx(), &tokens);
                fm_ui::preferences::show(ui.ctx(), s, &tokens);
            },
            s,
        );
    h.run();
    h.get_by_label("Media").click();
    h.run();
    h.get_by_label("When I open a video file");
    let radios = h.get_all_by_label("Play in Media pane").collect::<Vec<_>>();
    assert_eq!(radios.len(), 2, "one per kind");
    radios[1].click();
    h.run();
    assert_eq!(h.state().prefs.media_open_video, MediaOpenTarget::MediaPane);
    assert_eq!(
        h.state().prefs.media_open_audio,
        MediaOpenTarget::DefaultApp
    );
}

/// In a short pane the picture shrinks so the controls and media actions stay in view.
#[test]
fn a_short_preview_keeps_the_whole_player_in_view() {
    for poster in [false, true] {
        let mut s = Session::new().expect("session");
        s.prefs.preview_video_autoplay = false;
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("preview");
        let pane = s.layout.focused();
        s.poll();
        let (w, h) = (160u32, 90u32);
        s.previews.slots.get_mut(&pane).expect("slot").body = fm_engine::preview_cmds::Body::Static(
            std::sync::Arc::new(fm_preview::Preview::Meta(fm_preview::MetaDoc {
                rows: vec![("Duration".into(), "3:35".into())],
                hint: None,
                poster: poster.then(|| {
                    Box::new(fm_preview::ImageDoc {
                        width: w,
                        height: h,
                        frames: vec![(vec![128; (w * h * 4) as usize], 0)],
                        original: (w, h),
                        format: "JPEG",
                    })
                }),
                video: Some(fm_preview::VideoInfo {
                    path: "/sample/movie.mkv".into(),
                    range: None,
                    width: 1280,
                    height: 720,
                    fps: 25.0,
                    duration: 215.0,
                    audio: None,
                    audio_delay: 0.0,
                    tracks: Default::default(),
                }),
                audio: None,
            })),
        );
        s.poll();
        let tokens = Tokens::dark();
        let height = 380.0;
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, height))
            .build_ui_state(
                move |ui, s| {
                    fm_ui::theme::apply_to(ui.ctx(), &tokens);
                    egui::CentralPanel::default().show(ui, |ui| {
                        fm_ui::preview::show(ui, s, pane, &tokens);
                    });
                },
                s,
            );
        h.run();
        let bottom = h.get_by_label("Play folder").rect().bottom();
        assert!(bottom <= height, "poster {poster}: actions end at {bottom}");
    }
}

fn video_meta(path: &str, poster: bool, audio_tracks: usize) -> fm_engine::preview_cmds::Body {
    let (w, h) = (160u32, 90u32);
    let track = |n: usize| fm_preview::AudioTrack {
        label: format!("Track {n}"),
        format: None,
        delay: 0.0,
    };
    fm_engine::preview_cmds::Body::Static(std::sync::Arc::new(fm_preview::Preview::Meta(
        fm_preview::MetaDoc {
            rows: vec![("Duration".into(), "3:35".into())],
            hint: None,
            poster: poster.then(|| {
                Box::new(fm_preview::ImageDoc {
                    width: w,
                    height: h,
                    frames: vec![(vec![128; (w * h * 4) as usize], 0)],
                    original: (w, h),
                    format: "JPEG",
                })
            }),
            video: Some(fm_preview::VideoInfo {
                path: path.into(),
                range: None,
                width: 1280,
                height: 720,
                fps: 25.0,
                duration: 215.0,
                audio: None,
                audio_delay: 0.0,
                tracks: fm_preview::Tracks {
                    audio: (0..audio_tracks).map(track).collect(),
                    ..Default::default()
                },
            }),
            audio: None,
        },
    )))
}

/// Switching videos keeps the player in place through every step the app goes through:
/// loading, the new preview drawn before its player exists, then the player.
#[test]
fn switching_videos_in_preview_does_not_move_the_player() {
    let mut s = Session::new().expect("session");
    s.prefs.preview_video_autoplay = false;
    s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
        .expect("preview");
    let pane = s.layout.focused();
    s.poll();
    s.previews.slots.get_mut(&pane).expect("slot").body = video_meta("/a.mkv", true, 1);
    let tokens = Tokens::dark();
    // The engine ticks only when the test says so, like a new preview arriving mid-frame.
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                egui::CentralPanel::default().show(ui, |ui| {
                    fm_ui::preview::show(ui, s, pane, &tokens);
                });
            },
            s,
        );
    for _ in 0..10 {
        h.state_mut().poll();
        h.step();
    }
    let settled = h.get_by_label("Play folder").rect().top();
    let actions_top = |h: &Harness<'_, Session>| {
        // While loading or before the player exists there are no buttons: measure the
        // information card instead, which sits right below them.
        h.query_by_label("Play folder").map(|n| n.rect().top())
    };
    let card_top = |h: &Harness<'_, Session>| h.get_by_label("File information").rect().top();
    let card = card_top(&h);
    for (path, poster, tracks) in [
        ("/b.mkv", false, 1),
        ("/c.mkv", true, 3),
        ("/d.mkv", true, 1),
    ] {
        {
            let slot = h.state_mut().previews.slots.get_mut(&pane).expect("slot");
            let uri = fm_vfs::VfsUri::parse(path).expect("uri");
            slot.subject = Some(fm_vfs::Entry::new(uri, fm_vfs::EntryKind::File));
            slot.body = fm_engine::preview_cmds::Body::Loading;
        }
        h.state_mut().poll();
        h.step();
        // The preview arrives and is drawn once before the engine creates its player.
        h.state_mut()
            .previews
            .slots
            .get_mut(&pane)
            .expect("slot")
            .body = video_meta(path, poster, tracks);
        h.step();
        let before = card_top(&h);
        assert!(
            (before - card).abs() < 1.0,
            "{path} before its player: card at {before}, was at {card}"
        );
        for frame in 0..6 {
            h.state_mut().poll();
            h.step();
            let top = actions_top(&h).expect("actions");
            assert!(
                (top - settled).abs() < 1.0,
                "{path} frame {frame}: actions at {top}, were at {settled}"
            );
        }
    }
}

/// The poster is a still from ~10 s in: a video that is starting must not flash it before
/// its first frame (at 0:00). Only a paused, not yet started video shows it.
#[test]
fn the_poster_shows_only_for_a_paused_video() {
    for autoplay in [true, false] {
        let mut s = Session::new().expect("session");
        s.prefs.preview_video_autoplay = autoplay;
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("preview");
        let pane = s.layout.focused();
        s.poll();
        s.previews.slots.get_mut(&pane).expect("slot").body = video_meta("/a.mkv", true, 1);
        let tokens = Tokens::dark();
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui_state(
                move |ui, s: &mut Session| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        fm_ui::preview::show(ui, s, pane, &tokens);
                    });
                },
                s,
            );
        // The first frame shows the preview before its player exists, then the player.
        h.step();
        h.state_mut().poll();
        for _ in 0..4 {
            h.step();
        }
        let playing = h
            .state()
            .previews
            .slots
            .get(&pane)
            .and_then(|slot| slot.video.as_ref())
            .is_some_and(|v| v.playing);
        assert_eq!(playing, autoplay);
        let poster_loaded = h
            .ctx
            .tex_manager()
            .read()
            .allocated()
            .any(|(_, meta)| meta.name == "video-poster");
        assert_eq!(poster_loaded, !autoplay, "autoplay {autoplay}");
    }
}

fn image_body(width: u32, height: u32, shade: u8) -> fm_engine::preview_cmds::Body {
    fm_engine::preview_cmds::Body::Static(std::sync::Arc::new(fm_preview::Preview::Image(
        fm_preview::ImageDoc {
            width,
            height,
            frames: vec![(vec![shade; (width * height * 4) as usize], 0)],
            original: (width, height),
            format: "PNG",
        },
    )))
}

fn image_textures(h: &Harness<'_, Session>) -> Vec<egui::TextureId> {
    fn walk(shape: &egui::Shape, out: &mut Vec<egui::TextureId>) {
        match shape {
            egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                out.push(mesh.texture_id);
            }
            egui::Shape::Rect(rect) if rect.fill_texture_id() != egui::TextureId::default() => {
                out.push(rect.fill_texture_id());
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for clipped in &h.output().shapes {
        walk(&clipped.shape, &mut out);
    }
    out
}

/// Selecting another image shows that image: the texture cache must not mistake the next
/// preview for the previous one (a freed preview's address can be reused).
#[test]
fn selecting_another_image_shows_the_new_image() {
    let mut s = Session::new().expect("session");
    s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
        .expect("preview");
    let pane = s.layout.focused();
    s.poll();
    s.previews.slots.get_mut(&pane).expect("slot").body = image_body(40, 30, 50);
    let tokens = Tokens::dark();
    let mut h = Harness::builder()
        .with_size(egui::vec2(900.0, 600.0))
        .build_ui_state(
            move |ui, s: &mut Session| {
                egui::CentralPanel::default().show(ui, |ui| {
                    fm_ui::preview::show(ui, s, pane, &tokens);
                });
            },
            s,
        );
    h.run();
    let first = image_textures(&h);
    assert!(!first.is_empty(), "the first image is drawn");
    for (n, (w, height)) in [(30, 40), (40, 30), (64, 64)].into_iter().enumerate() {
        // Dropping the old body first makes its memory free for the new one.
        h.state_mut()
            .previews
            .slots
            .get_mut(&pane)
            .expect("slot")
            .body = fm_engine::preview_cmds::Body::Empty;
        h.state_mut()
            .previews
            .slots
            .get_mut(&pane)
            .expect("slot")
            .body = image_body(w, height, 60 + n as u8 * 40);
        h.run();
        let now = image_textures(&h);
        assert!(!now.is_empty());
        assert!(
            now.iter().all(|id| !first.contains(id)),
            "image {n} reuses the previous texture"
        );
    }
}
