//! Contextual playback keys (YouTube-style) for a focused Media pane, plus the
//! transport subset for a focused Preview pane.
//!
//! These are bare keys, so they are not in the global shortcut table: they are
//! consumed here only when the focused pane is Media/Preview and nothing else
//! (dialog, text field, terminal, focused slider) owns the keyboard. Every key
//! maps to one engine command; nothing here touches playback directly.

use egui::{Context, Event, Id, Key, Modifiers};
use fm_engine::Session;
use fm_layout::PaneKind;
use serde_json::{Value, json};

pub const SEEK_SMALL: f64 = 5.0;
pub const SEEK_LARGE: f64 = 10.0;
pub const VOLUME_STEP: f32 = 5.0;
pub const SPEED_STEP: f64 = 0.25;
pub const SPEED_MIN: f64 = 0.5;
pub const SPEED_MAX: f64 = 3.0;

/// `(keys, effect)` rows for the in-pane help.
pub const HELP: &[(&str, &str)] = &[
    ("Space / K", "Play or pause"),
    ("J / L", "Back / forward 10 s"),
    ("Left / Right", "Back / forward 5 s"),
    ("Up / Down", "Volume +5 / -5"),
    ("M", "Mute"),
    ("0-9", "Jump to 0-90% of the item"),
    ("Home / End", "Start / end of the item"),
    ("F / Esc", "Fullscreen / leave fullscreen"),
    ("N / P", "Next / previous item"),
    (", / .", "Previous / next frame (paused video)"),
    ("< / >", "Slower / faster (0.5x-3x)"),
    ("C", "Toggle subtitles"),
    ("G / H", "Subtitles 0.1 s earlier / later"),
];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KeyAction {
    Toggle,
    SeekBy(f64),
    /// Fraction of the duration, 0..=1.
    SeekFraction(f64),
    SeekStart,
    SeekEnd,
    VolumeBy(f32),
    Mute,
    Fullscreen,
    ExitFullscreen,
    Next,
    Prev,
    Step(i32),
    SpeedBy(f64),
    Subtitles,
    /// Steps of 100 ms; positive shows subtitles later.
    SubtitleDelay(i64),
}

/// Which playback the keys drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Media,
    PreviewVideo,
    PreviewAudio,
}

/// What the planner needs to know about the playback it drives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub target: Target,
    pub position: f64,
    pub duration: Option<f64>,
    pub volume: f32,
    pub speed: f64,
    pub fullscreen: bool,
}

impl KeyAction {
    /// Navigation keys that a focused widget (button, list, slider) also uses.
    fn widget_key(self) -> bool {
        matches!(
            self,
            Self::Toggle | Self::SeekBy(_) | Self::VolumeBy(_) | Self::SeekStart | Self::SeekEnd
        )
    }
}

/// The action for a key press. Ctrl/Alt/Cmd chords belong to the shortcut table.
pub fn action_for(key: Key, mods: Modifiers) -> Option<KeyAction> {
    if mods.ctrl || mods.alt || mods.command || mods.mac_cmd {
        return None;
    }
    use KeyAction::*;
    if mods.shift {
        return match key {
            Key::N => Some(Next),
            Key::P => Some(Prev),
            Key::Comma => Some(SpeedBy(-SPEED_STEP)),
            Key::Period => Some(SpeedBy(SPEED_STEP)),
            _ => None,
        };
    }
    let digit = [
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ]
    .iter()
    .position(|k| *k == key);
    if let Some(n) = digit {
        return Some(SeekFraction(n as f64 / 10.0));
    }
    Some(match key {
        Key::Space | Key::K => Toggle,
        Key::J => SeekBy(-SEEK_LARGE),
        Key::L => SeekBy(SEEK_LARGE),
        Key::ArrowLeft => SeekBy(-SEEK_SMALL),
        Key::ArrowRight => SeekBy(SEEK_SMALL),
        Key::ArrowUp => VolumeBy(VOLUME_STEP),
        Key::ArrowDown => VolumeBy(-VOLUME_STEP),
        Key::M => Mute,
        Key::Home => SeekStart,
        Key::End => SeekEnd,
        Key::F => Fullscreen,
        Key::Escape => ExitFullscreen,
        Key::N => Next,
        Key::P => Prev,
        Key::Comma => Step(-1),
        Key::Period => Step(1),
        Key::C => Subtitles,
        Key::G => SubtitleDelay(-1),
        Key::H => SubtitleDelay(1),
        _ => return None,
    })
}

fn command(target: Target, leaf: &str) -> &'static str {
    match (target, leaf) {
        (Target::Media, "seek") => "media.seek",
        (Target::Media, "volume") => "media.volume",
        (Target::Media, "mute") => "media.mute",
        (Target::PreviewVideo, "seek") => "preview.video.seek",
        (Target::PreviewVideo, "volume") => "preview.video.volume",
        (Target::PreviewVideo, "mute") => "preview.video.mute",
        (Target::PreviewVideo, _) => "preview.video.play",
        (Target::PreviewAudio, "seek") => "preview.audio.seek",
        (Target::PreviewAudio, "volume") => "preview.audio.volume",
        (Target::PreviewAudio, "mute") => "preview.audio.mute",
        (Target::PreviewAudio, _) => "preview.audio.play",
        (Target::Media, _) => "media.toggle",
    }
}

/// The command (and params) a key action runs, or `None` when it does not apply.
pub fn plan(action: KeyAction, view: &View) -> Option<(&'static str, Value)> {
    let media = view.target == Target::Media;
    let name = |leaf: &str| command(view.target, leaf);
    let seek_to = |to: f64| Some((name("seek"), json!({"to": to.max(0.0)})));
    match action {
        KeyAction::Toggle => Some((name("play"), Value::Null)),
        KeyAction::SeekBy(by) if media => Some(("media.seek", json!({"by": by}))),
        KeyAction::SeekBy(by) => {
            let to = view.position + by;
            seek_to(view.duration.map_or(to, |d| to.min(d)))
        }
        KeyAction::SeekFraction(f) => seek_to(view.duration? * f.clamp(0.0, 1.0)),
        KeyAction::SeekStart => seek_to(0.0),
        KeyAction::SeekEnd => seek_to(view.duration?),
        KeyAction::VolumeBy(by) => {
            let value = (view.volume + by).clamp(0.0, 100.0);
            Some((name("volume"), json!({"value": value})))
        }
        KeyAction::Mute => Some((name("mute"), Value::Null)),
        KeyAction::Fullscreen if media => Some(("media.fullscreen", Value::Null)),
        KeyAction::ExitFullscreen if media && view.fullscreen => {
            Some(("media.fullscreen", json!({"on": false})))
        }
        KeyAction::Next if media => Some(("media.next", Value::Null)),
        KeyAction::Prev if media => Some(("media.prev", Value::Null)),
        KeyAction::Step(frames) if media => Some(("media.step", json!({"frames": frames}))),
        KeyAction::SpeedBy(by) if media => {
            let value = ((view.speed + by).clamp(SPEED_MIN, SPEED_MAX) * 20.0).round() / 20.0;
            Some(("media.speed", json!({"value": value})))
        }
        KeyAction::Subtitles if media => Some(("media.track.subtitle", json!({"toggle": true}))),
        KeyAction::SubtitleDelay(step) if media => {
            Some(("media.subtitle.delay", json!({"step": step})))
        }
        _ => None,
    }
}

fn slider_focus_id() -> Id {
    Id::NULL.with("media-slider-focus")
}

/// Called by the pane each frame: a focused slider owns every key.
pub fn note_slider_focus(ctx: &Context, focused: bool) {
    ctx.data_mut(|d| d.insert_temp(slider_focus_id(), focused));
}

fn view(s: &Session) -> Option<View> {
    match s.focused_kind()? {
        PaneKind::Media => {
            let media = &s.media;
            Some(View {
                target: Target::Media,
                position: media.position,
                duration: media.duration(),
                volume: media.volume,
                speed: if media.video.is_some() {
                    media.speed
                } else {
                    1.0
                },
                fullscreen: s.media_fullscreen_active(),
            })
        }
        PaneKind::Preview => {
            let slot = s.previews.slots.get(&s.layout.focused())?;
            if let Some(video) = &slot.video {
                Some(View {
                    target: Target::PreviewVideo,
                    position: video.pos,
                    duration: Some(video.info.duration).filter(|d| *d > 0.0),
                    volume: video.volume,
                    speed: 1.0,
                    fullscreen: false,
                })
            } else {
                let audio = slot.audio.as_ref()?;
                Some(View {
                    target: Target::PreviewAudio,
                    position: audio.position,
                    duration: audio.doc.duration,
                    volume: audio.volume,
                    speed: 1.0,
                    fullscreen: false,
                })
            }
        }
        _ => None,
    }
}

/// Consume playback keys for the focused pane. `typing` is true while a dialog,
/// text field or form owns the keyboard. Returns the commands that ran.
pub fn handle(ctx: &Context, s: &mut Session, typing: bool) -> Vec<&'static str> {
    let mut ran = Vec::new();
    if typing || ctx.text_edit_focused() {
        return ran;
    }
    // Leaving fullscreen with Esc must work even when the pane is not otherwise driven.
    let Some(mut view) = view(s) else {
        return ran;
    };
    let slider: bool = ctx.data(|d| d.get_temp(slider_focus_id())).unwrap_or(false);
    // An open menu (the player's right-click menu, a dropdown) owns Esc and the arrows.
    if slider || egui::Popup::is_any_open(ctx) {
        return ran;
    }
    // The Media pane's controls are painted and take no keyboard navigation, so a stale focused
    // widget (a clicked button or playlist row) must not swallow Space and the arrows there.
    let widget_focused = view.target != Target::Media && ctx.memory(|m| m.focused()).is_some();
    let mut actions = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            // Only keys that do something here are taken: Esc outside fullscreen stays with
            // whatever else wants it.
            match action_for(*key, *modifiers) {
                Some(action)
                    if !(widget_focused && action.widget_key())
                        && plan(action, &view).is_some() =>
                {
                    actions.push(action);
                    false
                }
                _ => true,
            }
        });
    });
    for action in actions {
        let Some((id, params)) = plan(action, &view) else {
            continue;
        };
        if !s.command_enabled(id) {
            continue;
        }
        if s.dispatch(id, &params).is_ok() {
            // Later keys in the same frame see the new state (e.g. two seeks).
            if let Some(next) = self::view(s) {
                view = next;
            }
            ran.push(id);
        }
    }
    ran
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_view() -> View {
        View {
            target: Target::Media,
            position: 30.0,
            duration: Some(100.0),
            volume: 98.0,
            speed: 1.0,
            fullscreen: false,
        }
    }

    fn run(key: Key, mods: Modifiers, view: &View) -> Option<(&'static str, Value)> {
        plan(action_for(key, mods)?, view)
    }

    #[test]
    fn seek_keys_use_the_documented_increments() {
        let v = media_view();
        for (key, by) in [
            (Key::J, -10.0),
            (Key::L, 10.0),
            (Key::ArrowLeft, -5.0),
            (Key::ArrowRight, 5.0),
        ] {
            assert_eq!(
                run(key, Modifiers::NONE, &v),
                Some(("media.seek", json!({"by": by})))
            );
        }
    }

    #[test]
    fn volume_and_speed_are_clamped() {
        let v = media_view();
        assert_eq!(
            run(Key::ArrowUp, Modifiers::NONE, &v).expect("up").1,
            json!({"value": 100.0})
        );
        let low = View {
            volume: 2.0,
            speed: 0.5,
            ..v
        };
        assert_eq!(
            run(Key::ArrowDown, Modifiers::NONE, &low).expect("down").1,
            json!({"value": 0.0})
        );
        assert_eq!(
            run(Key::Comma, Modifiers::SHIFT, &low).expect("slower").1,
            json!({"value": 0.5})
        );
        let fast = View { speed: 2.9, ..v };
        assert_eq!(
            run(Key::Period, Modifiers::SHIFT, &fast).expect("faster").1,
            json!({"value": 3.0})
        );
    }

    #[test]
    fn digits_and_end_need_a_known_duration() {
        let v = media_view();
        assert_eq!(
            run(Key::Num5, Modifiers::NONE, &v).expect("five").1,
            json!({"to": 50.0})
        );
        assert_eq!(
            run(Key::End, Modifiers::NONE, &v).expect("end").1,
            json!({"to": 100.0})
        );
        let unknown = View {
            duration: None,
            ..v
        };
        assert!(run(Key::Num5, Modifiers::NONE, &unknown).is_none());
        assert!(run(Key::End, Modifiers::NONE, &unknown).is_none());
        assert!(run(Key::Home, Modifiers::NONE, &unknown).is_some());
    }

    #[test]
    fn modified_chords_are_left_to_the_shortcut_table() {
        let v = media_view();
        for mods in [Modifiers::CTRL, Modifiers::ALT, Modifiers::COMMAND] {
            assert!(action_for(Key::K, mods).is_none());
            assert!(action_for(Key::Num1, mods).is_none());
        }
        assert!(run(Key::Escape, Modifiers::NONE, &v).is_none());
        let full = View {
            fullscreen: true,
            ..v
        };
        assert_eq!(
            run(Key::Escape, Modifiers::NONE, &full).expect("leave").1,
            json!({"on": false})
        );
    }

    #[test]
    fn preview_gets_only_the_transport_subset() {
        let v = View {
            target: Target::PreviewVideo,
            ..media_view()
        };
        assert_eq!(
            run(Key::Space, Modifiers::NONE, &v).expect("toggle").0,
            "preview.video.play"
        );
        assert_eq!(
            run(Key::L, Modifiers::NONE, &v).expect("seek"),
            ("preview.video.seek", json!({"to": 40.0}))
        );
        for key in [Key::N, Key::P, Key::F, Key::C, Key::Comma] {
            assert!(run(key, Modifiers::NONE, &v).is_none(), "{key:?}");
        }
    }

    #[test]
    fn handle_consumes_each_key_once_and_respects_ownership() {
        let mut s = Session::new().expect("session");
        s.dispatch("media.enqueue", &json!({"uris": ["/music/one.mp3"]}))
            .expect("enqueue");
        let pane = s.media_pane().expect("media");
        s.layout.focus_pane(pane).expect("focus");
        let ctx = Context::default();
        let press = |key: Key| {
            let mut input = egui::RawInput::default();
            input.events.push(Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            });
            input
        };
        let run_frame = |s: &mut Session, input, typing| {
            let mut ran = Vec::new();
            let mut out = ctx.run_ui(input, |ui| {
                ran = handle(ui.ctx(), s, typing);
            });
            out.textures_delta.clear();
            ran
        };
        assert!(run_frame(&mut s, press(Key::M), true).is_empty());
        assert!(!s.media.muted);
        let ran = run_frame(&mut s, press(Key::M), false);
        assert_eq!(ran, vec!["media.mute"]);
        assert!(s.media.muted);
    }

    #[test]
    fn media_keys_work_while_a_widget_holds_focus() {
        let mut s = Session::new().expect("session");
        s.dispatch("media.enqueue", &json!({"uris": ["/music/one.mp3"]}))
            .expect("enqueue");
        let pane = s.media_pane().expect("media");
        s.layout.focus_pane(pane).expect("focus");
        let ctx = Context::default();
        let focus = Id::NULL.with("stale-focus");
        let mut ran = Vec::new();
        for frame in 0..2 {
            let mut input = egui::RawInput::default();
            if frame == 1 {
                input.events.push(Event::Key {
                    key: Key::ArrowUp,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
            let mut out = ctx.run_ui(input, |ui| {
                ui.ctx().memory_mut(|m| m.request_focus(focus));
                if frame == 1 {
                    ran = handle(ui.ctx(), &mut s, false);
                }
            });
            out.textures_delta.clear();
        }
        assert_eq!(ran, vec!["media.volume"]);
    }
}
