//! Embedded audio and subtitle track selection for the current Media video.

use fm_vfs::FmError;

use crate::{Session, file_cmds::object, registry::CommandSpec};

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn has_audio_tracks(s: &Session) -> bool {
    s.media.video.as_ref().is_some_and(|video| {
        video
            .info
            .tracks
            .audio
            .iter()
            .any(|track| track.format.is_some())
    })
}

fn has_subtitles(s: &Session) -> bool {
    s.media
        .video
        .as_ref()
        .is_some_and(|video| !video.info.tracks.subtitles.is_empty())
}

fn track_index(value: &serde_json::Value) -> Result<usize, FmError> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value < fm_preview::MAX_TRACKS)
        .ok_or_else(|| bad("track index must be a small non-negative integer"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.track.audio",
            label: "Audio Track",
            menu: &["Media", "Audio track"],
            shortcut: None,
            params: "{index: number}: embedded audio track of the current video, from 0",
            enabled: has_audio_tracks,
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("index: audio track required"))?;
                if map.keys().any(|key| key != "index") {
                    return Err(bad("only index is accepted"));
                }
                let index = track_index(map.get("index").ok_or_else(|| bad("index required"))?)?;
                let video = s
                    .media
                    .video
                    .as_mut()
                    .ok_or_else(|| bad("no video with audio tracks is loaded"))?;
                let track = video
                    .info
                    .tracks
                    .audio
                    .get(index)
                    .ok_or_else(|| bad("audio track out of range"))?;
                if track.format.is_none() {
                    return Err(bad("this audio track cannot be decoded"));
                }
                if video.info.tracks.selected_audio == index {
                    return Ok(());
                }
                video.info.tracks.selected_audio = index;
                if video.playing {
                    // Restarts decoding at the retained position with the new stream.
                    video.restart();
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.track.subtitle",
            label: "Subtitle Track",
            menu: &["Media", "Subtitles"],
            shortcut: None,
            params: "{index: number|null} selects an embedded subtitle track or turns subtitles off; {toggle: true} switches between off and the last (else first) track",
            enabled: has_subtitles,
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("index or toggle required"))?;
                if map.keys().any(|key| key != "index" && key != "toggle") {
                    return Err(bad("only index or toggle is accepted"));
                }
                if map.contains_key("index") == map.contains_key("toggle") {
                    return Err(bad("supply exactly one of index or toggle"));
                }
                let count = s
                    .media
                    .video
                    .as_ref()
                    .map(|video| video.info.tracks.subtitles.len())
                    .filter(|count| *count > 0)
                    .ok_or_else(|| bad("the current video has no subtitle tracks"))?;
                let target = if let Some(toggle) = map.get("toggle") {
                    if toggle.as_bool() != Some(true) {
                        return Err(bad("toggle must be true"));
                    }
                    match s.media.subtitle_track {
                        Some(_) => None,
                        None => Some(s.media.last_subtitle.filter(|i| *i < count).unwrap_or(0)),
                    }
                } else {
                    match map.get("index") {
                        Some(serde_json::Value::Null) => None,
                        Some(value) => {
                            let index = track_index(value)?;
                            if index >= count {
                                return Err(bad("subtitle track out of range"));
                            }
                            Some(index)
                        }
                        None => return Err(bad("index required")),
                    }
                };
                if target.is_some() {
                    s.media.last_subtitle = target;
                }
                s.media.subtitle_track = target;
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video_cmds::VideoState;
    use fm_media::AudioFormat;
    use fm_preview::{AudioTrack, SubtitleTrack, Tracks, VideoInfo};
    use serde_json::json;

    fn session(subtitles: usize) -> Session {
        let mut s = Session::new().expect("session");
        s.dispatch("media.enqueue", &json!({"uris": ["/video/movie.mkv"]}))
            .expect("enqueue");
        s.media.queue.select(0).expect("current");
        let format = AudioFormat::new(48_000, 2).expect("format");
        s.media.video = Some(VideoState::new(
            VideoInfo {
                path: "/video/movie.mkv".into(),
                width: 320,
                height: 180,
                fps: 25.0,
                duration: 10.0,
                audio: Some(format),
                audio_delay: 0.0,
                tracks: Tracks {
                    audio: vec![
                        AudioTrack {
                            label: "English".into(),
                            format: Some(format),
                            delay: 0.0,
                        },
                        AudioTrack {
                            label: "Broken".into(),
                            format: None,
                            delay: 0.0,
                        },
                        AudioTrack {
                            label: "Deutsch".into(),
                            format: Some(format),
                            delay: 0.0,
                        },
                    ],
                    subtitles: (0..subtitles)
                        .map(|i| SubtitleTrack {
                            label: format!("sub {i}"),
                        })
                        .collect(),
                    selected_audio: 0,
                },
            },
            false,
        ));
        s
    }

    fn selected(s: &Session) -> usize {
        s.media
            .video
            .as_ref()
            .expect("video")
            .info
            .tracks
            .selected_audio
    }

    #[test]
    fn audio_track_switch_keeps_position_and_playback_state() {
        let mut s = session(0);
        s.media.video.as_mut().expect("video").pos = 4.0;
        s.dispatch("media.track.audio", &json!({"index": 2}))
            .expect("switch");
        assert_eq!(selected(&s), 2);
        assert_eq!(s.media.video.as_ref().expect("video").pos, 4.0);
        assert!(!s.media.playing);
        s.media.video.as_mut().expect("video").playing = true;
        s.dispatch("media.track.audio", &json!({"index": 0}))
            .expect("switch while playing");
        let video = s.media.video.as_ref().expect("video");
        assert_eq!(video.info.tracks.selected_audio, 0);
        assert!(video.playing && !video.player_is_idle());
        assert_eq!(video.pos, 4.0);
    }

    #[test]
    fn bad_track_params_are_errors_and_change_nothing() {
        let mut s = session(2);
        for (command, params) in [
            ("media.track.audio", json!({"index": 1})),
            ("media.track.audio", json!({"index": 3})),
            ("media.track.audio", json!({"index": -1})),
            ("media.track.audio", json!({"index": 1.5})),
            ("media.track.audio", json!({"index": "1"})),
            ("media.track.audio", json!({"index": 99999999999u64})),
            ("media.track.audio", json!({"index": 0, "x": 1})),
            ("media.track.audio", json!(null)),
            ("media.track.subtitle", json!({"index": 2})),
            ("media.track.subtitle", json!({"index": -1})),
            ("media.track.subtitle", json!({"index": "0"})),
            ("media.track.subtitle", json!({"toggle": false})),
            ("media.track.subtitle", json!({"toggle": true, "index": 0})),
            ("media.track.subtitle", json!({})),
            ("media.track.subtitle", json!(null)),
        ] {
            assert!(s.dispatch(command, &params).is_err(), "{command} {params}");
            assert_eq!(selected(&s), 0);
            assert_eq!(s.media.subtitle_track, None);
        }
    }

    #[test]
    fn subtitle_toggle_selects_first_then_remembers_the_last_choice() {
        let mut s = session(3);
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("first");
        assert_eq!(s.media.subtitle_track, Some(0));
        s.dispatch("media.track.subtitle", &json!({"index": 2}))
            .expect("choose");
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("off");
        assert_eq!(s.media.subtitle_track, None);
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("on again");
        assert_eq!(s.media.subtitle_track, Some(2));
        s.dispatch("media.track.subtitle", &json!({"index": null}))
            .expect("off");
        assert_eq!(s.media.subtitle_track, None);
    }

    #[test]
    fn commands_are_disabled_without_tracks() {
        let s = session(0);
        assert!(!s.command_enabled("media.track.subtitle"));
        assert!(s.command_enabled("media.track.audio"));
        let mut s = s;
        s.media.video = None;
        assert!(!s.command_enabled("media.track.audio"));
        assert!(
            s.dispatch("media.track.audio", &json!({"index": 0}))
                .is_err()
        );
    }
}
