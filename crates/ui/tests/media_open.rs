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
