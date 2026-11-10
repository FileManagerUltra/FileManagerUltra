//! Video-only Media controls; decoding uses the shared worker-backed player.

use fm_vfs::FmError;

use crate::{Session, file_cmds::object, registry::CommandSpec};

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn can_step(s: &Session) -> bool {
    !s.media.playing
        && s.media
            .video
            .as_ref()
            .is_some_and(|video| video.info.fps > 0.0)
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.speed",
            label: "Playback Speed",
            menu: &["Media", "Speed"],
            shortcut: None,
            params: "{value: number}: video speed from 0.5 to 3 in 0.05 increments",
            enabled: |s| s.media.is_video(),
            run: |s, params| {
                let map =
                    object(params)?.ok_or_else(|| bad("value: speed from 0.5 to 3 required"))?;
                if map.keys().any(|key| key != "value") {
                    return Err(bad("only value is accepted"));
                }
                let value = map
                    .get("value")
                    .and_then(|value| value.as_f64())
                    .filter(|value| value.is_finite() && (0.5..=3.0).contains(value))
                    .ok_or_else(|| bad("speed must be between 0.5 and 3"))?;
                let steps = value * 20.0;
                if (steps - steps.round()).abs() > 0.000_1 {
                    return Err(bad("speed must use 0.05 increments"));
                }
                if !s.media.is_video() {
                    return Err(bad("video speed is unavailable for this item"));
                }
                s.media.speed = value;
                if let Some(video) = &mut s.media.video {
                    video.speed = value;
                    if video.playing {
                        video.restart();
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.video.fit",
            label: "Fit Media Video Decode Size",
            menu: &["Media", "Fit video size"],
            shortcut: None,
            params: "{width: number}: available video width in physical pixels",
            enabled: |s| s.media.is_video(),
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("width: positive number required"))?;
                if map.keys().any(|key| key != "width") {
                    return Err(bad("only width is accepted"));
                }
                let width = map
                    .get("width")
                    .and_then(|value| value.as_f64())
                    .filter(|width| width.is_finite() && *width >= 1.0)
                    .ok_or_else(|| bad("width: positive finite number required"))?;
                if !s.media.is_video() {
                    return Err(bad("the current Media item is not a video"));
                }
                let width = crate::video_cmds::width_bucket(width as f32);
                s.media.video_width = width;
                if let Some(video) = &mut s.media.video
                    && video.width != Some(width)
                {
                    video.width = Some(width);
                    if video.playing {
                        video.restart();
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.step",
            label: "Step Video Frame",
            menu: &["Media", "Step video frame"],
            shortcut: None,
            params: "{frames?: -1|1}: step one frame while paused; forward by default",
            enabled: can_step,
            run: |s, params| {
                let map = object(params)?;
                if map.is_some_and(|map| map.keys().any(|key| key != "frames")) {
                    return Err(bad("only frames is accepted"));
                }
                let frames = map
                    .and_then(|map| map.get("frames"))
                    .map(|value| {
                        value
                            .as_i64()
                            .filter(|frames| matches!(frames, -1 | 1))
                            .ok_or_else(|| bad("frames must be -1 or 1"))
                    })
                    .transpose()?
                    .unwrap_or(1);
                if !can_step(s) {
                    return Err(bad("pause a video before stepping frames"));
                }
                let video = s
                    .media
                    .video
                    .as_mut()
                    .ok_or_else(|| bad("no current video"))?;
                let fps = video.info.fps;
                let last = if video.info.duration > 0.0 {
                    ((video.info.duration * fps).ceil() - 1.0).max(0.0) / fps
                } else {
                    f64::MAX
                };
                let to = ((video.pos * fps).round() + frames as f64) / fps;
                let to = to.clamp(0.0, last);
                video.seek(to);
                s.media.position = to;
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio_owner::AudioOwner, registry::Params, video_cmds::VideoState};
    use fm_vfs::VfsUri;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn prepared() -> Session {
        let mut s = Session::new().expect("session");
        s.dispatch("media.enqueue", &json!({"uris": ["/video/movie.mkv"]}))
            .expect("enqueue");
        s.media.queue.select(0).expect("current");
        s.media.video = Some(VideoState::new(
            fm_preview::VideoInfo {
                path: "/video/movie.mkv".into(),
                width: 320,
                height: 180,
                fps: 25.0,
                duration: 10.0,
                audio: None,
                audio_delay: 0.0,
                tracks: Default::default(),
            },
            false,
        ));
        s
    }

    fn wait(s: &mut Session, ready: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        while !ready(s) {
            s.poll();
            assert!(
                Instant::now() < deadline,
                "video did not settle: {:?}",
                s.media.error
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn clip() -> Option<(tempfile::TempDir, VfsUri)> {
        let ffmpeg = fm_preview::video::find_tool("ffmpeg")?;
        fm_preview::video::find_tool("ffprobe")?;
        let dir = tempfile::tempdir().expect("directory");
        let path = dir.path().join("clip.mkv");
        assert!(
            std::process::Command::new(ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc=size=160x90:rate=10:duration=1",
                    "-c:v",
                    "mpeg4",
                    "-pix_fmt",
                    "yuv420p",
                    "-threads",
                    "1",
                    "-y",
                ])
                .arg(&path)
                .status()
                .expect("ffmpeg")
                .success()
        );
        let uri = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        Some((dir, uri))
    }

    #[test]
    fn stepping_is_paused_clamped_and_rejects_bad_params_without_mutation() {
        let mut s = prepared();
        s.dispatch("media.step", &json!({"frames": -1}))
            .expect("clamp start");
        assert_eq!(s.media.position, 0.0);
        s.dispatch("media.step", &Params::Null).expect("forward");
        assert_eq!(s.media.position, 0.04);
        s.dispatch("media.step", &json!({"frames": -1}))
            .expect("back");
        assert_eq!(s.media.position, 0.0);
        s.dispatch("media.seek", &json!({"to": 9.99}))
            .expect("seek");
        s.dispatch("media.step", &json!({"frames": 1}))
            .expect("clamp end");
        assert!((s.media.position - 9.96).abs() < 0.0001);
        assert!(!s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::None);
        assert_eq!(s.media.queue.items().len(), 1);
        for (command, params) in [
            ("media.step", json!({"frames": 0})),
            ("media.step", json!({"frames": 2})),
            ("media.step", json!({"frames": 1.5})),
            ("media.step", json!({"frames": "1"})),
            ("media.step", json!({"unknown": true})),
            ("media.video.fit", json!({"width": -1})),
            ("media.video.fit", json!({"width": "640"})),
            ("media.video.fit", json!({"width": 640, "unknown": true})),
        ] {
            let before = s.media.position;
            assert!(s.dispatch(command, &params).is_err());
            assert_eq!(s.media.position, before);
            assert!(!s.media.playing);
        }
        s.media.playing = true;
        assert!(!s.command_enabled("media.step"));
        assert!(s.dispatch("media.step", &Params::Null).is_err());
    }

    #[test]
    fn speed_validates_finite_range_and_five_hundredths_steps() {
        let mut s = prepared();
        for value in [0.5, 0.75, 1.0, 1.5, 2.95, 3.0] {
            s.dispatch("media.speed", &json!({"value": value}))
                .expect("speed");
            assert_eq!(s.media.speed, value);
            assert_eq!(s.media.video.as_ref().expect("video").speed, value);
        }
        for params in [
            json!({"value": 0.0}),
            json!({"value": 3.01}),
            json!({"value": 1.333}),
            json!({"value": "1.5"}),
            json!({"value": null}),
            json!({"value": 1.5, "extra": 1}),
            json!(null),
            json!(f64::NAN),
        ] {
            let before = s.media.speed;
            assert!(s.dispatch("media.speed", &params).is_err());
            assert_eq!(s.media.speed, before);
            assert_eq!(s.media.video.as_ref().expect("video").speed, before);
        }
        s.media.queue = fm_media::Queue::default();
        s.media
            .queue
            .enqueue(VfsUri::parse("/audio/song.mp3").expect("uri"))
            .expect("queue");
        s.media.queue.select(0).expect("current");
        assert!(!s.command_enabled("media.speed"));
        assert!(s.dispatch("media.speed", &json!({"value": 2.0})).is_err());
    }

    #[test]
    fn changing_speed_restarts_the_current_video_without_losing_position_or_owner() {
        let Some((_dir, uri)) = clip() else { return };
        let mut s = Session::new().expect("session");
        s.dispatch("media.play", &json!({"uris": [uri.to_string()]}))
            .expect("play");
        wait(&mut s, |s| {
            s.media
                .video
                .as_ref()
                .is_some_and(|video| video.frame.is_some())
        });
        s.dispatch("media.seek", &json!({"to": 0.2})).expect("seek");
        s.dispatch("media.speed", &json!({"value": 2.0}))
            .expect("speed");
        assert!(s.media.playing);
        assert_eq!(s.media.position, 0.2);
        assert_eq!(s.audio_owner, AudioOwner::Media);
        assert_eq!(s.media.video.as_ref().expect("video").speed, 2.0);
        wait(&mut s, |s| s.media.position > 0.35);
        s.media_stop();
    }

    #[test]
    fn video_stops_on_close_kind_switch_and_workspace_change_even_with_background_audio() {
        for action in ["pane.close", "pane.kind.set", "workspace.new"] {
            for background in [false, true] {
                let mut s = prepared();
                s.prefs.media_background_audio = background;
                let pane = s.media_pane().expect("pane");
                s.media.playing = true;
                s.media.video.as_mut().expect("video").playing = true;
                s.audio_owner = AudioOwner::Media;
                let params = if action == "pane.kind.set" {
                    json!({"kind": "files", "pane": pane})
                } else if action == "pane.close" {
                    json!({"pane": pane})
                } else {
                    Params::Null
                };
                s.dispatch(action, &params).expect("leave video");
                assert!(!s.media.playing, "{action}");
                assert!(s.media.video.as_ref().expect("video").player_is_idle());
                assert_eq!(s.audio_owner, AudioOwner::None);
                assert_eq!(s.media.queue.items().len(), 1);
            }
        }
    }

    #[test]
    fn real_video_probes_while_paused_steps_renders_and_advances_without_removing_items() {
        let Some((_dir, uri)) = clip() else { return };
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.play",
            &json!({"uris": [uri.to_string(), uri.to_string()]}),
        )
        .expect("play");
        s.dispatch("media.pause", &Params::Null)
            .expect("pause before probe finishes");
        wait(&mut s, |s| s.media.video.is_some());
        assert!(!s.media.playing);
        assert!(s.media.video.as_ref().expect("video").player_is_idle());
        s.dispatch("media.video.fit", &json!({"width": 100_000}))
            .expect("fit");
        assert_eq!(
            s.media.video.as_ref().expect("video").width,
            Some(fm_preview::player::MAX_WIDTH)
        );
        s.dispatch("media.step", &Params::Null).expect("step");
        wait(&mut s, |s| {
            s.media
                .video
                .as_ref()
                .is_some_and(|v| v.frame.is_some() && v.player_is_idle())
        });
        assert!((s.media.position - 0.1).abs() < 0.0001);
        assert!(!s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::None);
        let frame = s
            .media
            .video
            .as_ref()
            .expect("video")
            .frame
            .as_ref()
            .expect("frame");
        assert_eq!((frame.width, frame.height), (160, 90));
        s.dispatch("media.toggle", &Params::Null).expect("resume");
        wait(&mut s, |s| s.media.position > 0.2);
        s.claim_audio(AudioOwner::PreviewAudio(s.layout.focused()));
        assert!(!s.media.playing);
        assert!(s.media.video.as_ref().expect("video").player_is_idle());
        s.dispatch("media.toggle", &Params::Null).expect("resume");
        wait(&mut s, |s| !s.media.playing);
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert_eq!(s.media.queue.items().len(), 2);
        assert!(s.media.error.is_none(), "{:?}", s.media.error);
        assert_eq!(s.audio_owner, AudioOwner::None);
    }

    #[test]
    fn replaced_probe_cannot_attach_the_previous_video_and_failures_are_reported_once() {
        let Some((dir, uri)) = clip() else { return };
        let mut s = Session::new().expect("session");
        s.dispatch("media.play", &json!({"uris": [uri.to_string()]}))
            .expect("start");
        let missing = dir
            .path()
            .join("missing.mkv")
            .to_string_lossy()
            .into_owned();
        s.dispatch("media.play", &json!({"uris": [missing]}))
            .expect("replace");
        s.media.queue.set_repeat(fm_media::RepeatMode::One);
        wait(&mut s, |s| !s.media.playing);
        assert!(s.media.video.is_none());
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert_eq!(s.messages.len(), 1);
        for _ in 0..3 {
            s.poll();
        }
        assert_eq!(s.messages.len(), 1);
        assert_eq!(s.media.queue.items().len(), 2);
        let mut video = VideoState::new(
            fm_preview::VideoInfo {
                path: s
                    .media
                    .queue
                    .current()
                    .expect("item")
                    .local_path_string()
                    .expect("path"),
                width: 160,
                height: 90,
                fps: 10.0,
                duration: 1.0,
                audio: None,
                audio_delay: 0.0,
                tracks: Default::default(),
            },
            false,
        );
        video.width = Some(160);
        s.media.video = Some(video);
        s.dispatch("media.toggle", &Params::Null)
            .expect("retry decoder");
        wait(&mut s, |s| !s.media.playing);
        assert_eq!(s.messages.len(), 2);
        for _ in 0..3 {
            s.poll();
        }
        assert_eq!(
            s.messages.len(),
            2,
            "a decoder failure is reported only once"
        );
    }
}
