//! Where activating an audio or video file goes (default app or the Media pane), and the
//! one-time hint that tells users the Media pane exists (capability `media-open-preference`).

use fm_vfs::{FmError, VfsUri};
use serde_json::{Value, json};

use crate::file_cmds::object;
use crate::messages::MessageAction;
use crate::prefs::{MediaOpenTarget, is_hint_id};
use crate::registry::CommandSpec;
use crate::session::Session;

/// Id of the "you can also play media here" hint in `Prefs::hints_seen`.
pub const MEDIA_OPEN_HINT: &str = "media-open";
const HINT_TEXT: &str = "You can also play audio and video here.";
/// How long a file opened elsewhere keeps its (still loading) Preview from autoplaying.
const HOLD_AUTOPLAY: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Audio,
    Video,
}

impl MediaKind {
    fn key(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
}

/// The kind of a local file the Media pane plays on activation. Remote files, disc images,
/// playlists and anything unsupported are `None` and keep opening in the default app.
pub fn media_open_kind(uri: &VfsUri) -> Option<MediaKind> {
    uri.local_path()?;
    let name = uri.file_name()?;
    if fm_preview::video::is_disc_image(name) {
        return None;
    }
    crate::media_cmds::validate_media_source(uri).ok()?;
    Some(if crate::media_cmds::is_video(uri) {
        MediaKind::Video
    } else {
        MediaKind::Audio
    })
}

impl Session {
    /// `uri` was just opened elsewhere (default app or Media pane): pause any Preview
    /// pane playing it, so the two players never run at once.
    pub fn pause_previews_of(&mut self, uri: &VfsUri) {
        use crate::audio_owner::AudioOwner;
        // The first click of the double-click may still be loading the preview.
        self.opened_elsewhere = Some((uri.clone(), std::time::Instant::now()));
        let panes: Vec<fm_layout::PaneId> = self
            .previews
            .slots
            .iter()
            .filter(|(_, slot)| slot.subject.as_ref().is_some_and(|e| e.uri == *uri))
            .map(|(pane, _)| *pane)
            .collect();
        for pane in panes {
            if let Some(slot) = self.previews.slots.get_mut(&pane) {
                if let Some(video) = slot.video.as_mut().filter(|v| v.playing) {
                    video.pause();
                }
                if let Some(audio) = slot.audio.as_mut().filter(|a| a.playing) {
                    audio.pause();
                }
            }
            self.release_audio(AudioOwner::PreviewVideo(pane));
            self.release_audio(AudioOwner::PreviewAudio(pane));
        }
    }

    /// Whether a Preview of `uri` may start playing by itself: not right after the file was
    /// opened in another player.
    pub(crate) fn preview_may_autoplay(&self, uri: Option<&VfsUri>) -> bool {
        !self
            .opened_elsewhere
            .as_ref()
            .is_some_and(|(held, at)| uri == Some(held) && at.elapsed() < HOLD_AUTOPLAY)
    }

    /// The activation target the preferences choose for `kind`.
    pub fn media_open_target(&self, kind: MediaKind) -> MediaOpenTarget {
        match kind {
            MediaKind::Audio => self.prefs.media_open_audio,
            MediaKind::Video => self.prefs.media_open_video,
        }
    }

    /// After `uri` went to the default app: show the one-time hint when it is media the
    /// Media pane could have played. Marked seen the moment it is shown, so it never returns.
    pub fn offer_media_open_hint(&mut self, uri: &VfsUri) {
        let Some(kind) = media_open_kind(uri) else {
            return;
        };
        if self.media_open_target(kind) != MediaOpenTarget::DefaultApp
            || self.prefs.hint_seen(MEDIA_OPEN_HINT)
        {
            return;
        }
        self.prefs.mark_hint_seen(MEDIA_OPEN_HINT);
        let play = ("media.play".to_string(), json!({"uris": [uri.to_string()]}));
        self.messages.push_hint(
            HINT_TEXT,
            MEDIA_OPEN_HINT,
            vec![
                MessageAction {
                    label: "Play in Media pane".into(),
                    commands: vec![play.clone()],
                },
                MessageAction {
                    label: "Always play here".into(),
                    commands: vec![
                        (
                            "prefs.media_open".to_string(),
                            json!({"kind": kind.key(), "target": "media"}),
                        ),
                        play,
                    ],
                },
                MessageAction {
                    label: "Don't show again".into(),
                    commands: vec![("hint.dismiss".to_string(), json!({ "id": MEDIA_OPEN_HINT }))],
                },
            ],
        );
    }
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn field<'a>(map: &'a serde_json::Map<String, Value>, key: &str) -> Result<&'a str, FmError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(&format!("{key} must be a string")))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "prefs.media_open",
            label: "Open Media Files In…",
            menu: &["Edit", "Preferences", "Open media files in"],
            shortcut: None,
            params: "{kind: \"audio\"|\"video\"|\"both\", target: \"default\"|\"media\"}: what activating an audio or video file does",
            enabled: |_| true,
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("expected {kind, target}"))?;
                if map.keys().any(|k| k != "kind" && k != "target") {
                    return Err(bad("only kind and target are accepted"));
                }
                let target = match field(map, "target")? {
                    "default" => MediaOpenTarget::DefaultApp,
                    "media" => MediaOpenTarget::MediaPane,
                    _ => return Err(bad("target must be default or media")),
                };
                let (audio, video) = match field(map, "kind")? {
                    "audio" => (true, false),
                    "video" => (false, true),
                    "both" => (true, true),
                    _ => return Err(bad("kind must be audio, video or both")),
                };
                if audio {
                    s.prefs.media_open_audio = target;
                }
                if video {
                    s.prefs.media_open_video = target;
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "hint.dismiss",
            label: "Dismiss Hint",
            menu: &["Activity", "Dismiss hint"],
            shortcut: None,
            params: "{id: string}: close a one-time hint in the Activity panel",
            enabled: |_| true,
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("expected {id}"))?;
                if map.keys().any(|k| k != "id") {
                    return Err(bad("only id is accepted"));
                }
                let id = field(map, "id")?;
                if !is_hint_id(id) {
                    return Err(bad("unknown hint"));
                }
                s.prefs.mark_hint_seen(id);
                s.messages.dismiss_hint(id);
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    fn uri(path: &str) -> VfsUri {
        VfsUri::parse(path).expect("uri")
    }

    #[test]
    fn media_kinds_are_local_playable_files_only() {
        assert_eq!(media_open_kind(&uri("/m/song.mp3")), Some(MediaKind::Audio));
        assert_eq!(media_open_kind(&uri("/m/book.m4b")), Some(MediaKind::Audio));
        assert_eq!(media_open_kind(&uri("/m/film.mkv")), Some(MediaKind::Video));
        for other in [
            "/m/list.m3u",
            "/m/list.pls",
            "/m/Movie.iso",
            "/m/notes.txt",
            "sftp://host/m/song.mp3",
        ] {
            assert_eq!(media_open_kind(&uri(other)), None, "{other}");
        }
    }

    #[test]
    fn preference_command_sets_each_kind_and_rejects_bad_params() {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "prefs.media_open",
            &json!({"kind": "audio", "target": "media"}),
        )
        .expect("audio");
        assert_eq!(s.prefs.media_open_audio, MediaOpenTarget::MediaPane);
        assert_eq!(s.prefs.media_open_video, MediaOpenTarget::DefaultApp);
        s.dispatch(
            "prefs.media_open",
            &json!({"kind": "both", "target": "default"}),
        )
        .expect("both");
        assert_eq!(s.prefs.media_open_audio, MediaOpenTarget::DefaultApp);
        let before = s.prefs.clone();
        for bad in [
            Value::Null,
            json!("audio"),
            json!({}),
            json!({"kind": "audio"}),
            json!({"kind": "music", "target": "media"}),
            json!({"kind": "audio", "target": "vlc"}),
            json!({"kind": 1, "target": "media"}),
            json!({"kind": "audio", "target": "media", "extra": true}),
        ] {
            assert!(s.dispatch("prefs.media_open", &bad).is_err(), "{bad}");
        }
        assert_eq!(s.prefs, before);
    }

    #[test]
    fn the_hint_is_offered_once_for_media_and_never_with_the_media_pane_target() {
        let mut s = Session::new().expect("session");
        s.offer_media_open_hint(&uri("/m/notes.txt"));
        assert!(s.messages.is_empty(), "not media");
        s.prefs.media_open_audio = MediaOpenTarget::MediaPane;
        s.offer_media_open_hint(&uri("/m/song.mp3"));
        assert!(s.messages.is_empty(), "already playing in the app");
        s.prefs.media_open_audio = MediaOpenTarget::DefaultApp;
        s.offer_media_open_hint(&uri("/m/song.mp3"));
        assert!(s.messages.newest_is_hint());
        assert!(s.prefs.hint_seen(MEDIA_OPEN_HINT), "marked when shown");
        let hint = s.messages.recent().next().expect("hint").clone();
        assert_eq!(hint.actions.len(), 3);
        assert_eq!(hint.actions[1].commands[0].0, "prefs.media_open");
        s.offer_media_open_hint(&uri("/m/film.mkv"));
        assert_eq!(s.messages.len(), 1, "shown once");

        s.dispatch("hint.dismiss", &json!({ "id": MEDIA_OPEN_HINT }))
            .expect("dismiss");
        assert!(s.messages.is_empty());
        for bad in [
            Value::Null,
            json!({}),
            json!({"id": 3}),
            json!({"id": "Not An Id"}),
            json!({"id": "x".repeat(200)}),
            json!({"id": "media-open", "more": 1}),
        ] {
            assert!(s.dispatch("hint.dismiss", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn always_play_here_sets_the_preference_through_the_registry() {
        let mut s = Session::new().expect("session");
        s.offer_media_open_hint(&uri("/m/film.mkv"));
        let id = s.messages.recent().next().expect("hint").id;
        let commands = s.messages.use_action(id, 1).expect("always");
        let (command, params) = &commands[0];
        s.dispatch(command, params).expect("preference");
        assert_eq!(s.prefs.media_open_video, MediaOpenTarget::MediaPane);
        assert_eq!(s.prefs.media_open_audio, MediaOpenTarget::DefaultApp);
    }
}
