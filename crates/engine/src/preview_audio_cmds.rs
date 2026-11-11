//! Single-file audio in Preview. Files and devices are opened only by fm-media's worker.

use crate::audio_owner::AudioOwner;
use fm_media::{AudioPlayer, Gain, PlaybackState};
use fm_preview::{Preview, audio::AudioDoc};
use fm_vfs::FmError;

use crate::Session;
use crate::file_cmds::object;
use crate::preview_cmds::Body;
use crate::registry::{CommandSpec, Params};

#[derive(Debug)]
pub struct AudioState {
    pub doc: AudioDoc,
    pub playing: bool,
    pub position: f64,
    pub volume: f32,
    pub muted: bool,
    pub error: Option<String>,
    gain: Gain,
    player: Option<AudioPlayer>,
}

impl AudioState {
    fn new(doc: AudioDoc) -> Self {
        Self {
            doc,
            playing: false,
            position: 0.0,
            volume: 100.0,
            muted: false,
            error: None,
            gain: Gain::default(),
            player: None,
        }
    }

    fn play(&mut self, on: bool) -> Result<(), FmError> {
        if on && self.player.is_none() {
            let player = AudioPlayer::start(self.doc.path.clone(), self.gain.clone())?;
            if self.position > 0.0 {
                player.seek(self.position)?;
            }
            self.player = Some(player);
        }
        if let Some(player) = &self.player {
            player.set_paused(!on);
        }
        self.playing = on;
        self.error = None;
        Ok(())
    }

    pub(crate) fn pause(&mut self) {
        self.playing = false;
        if let Some(player) = &self.player {
            player.set_paused(true);
        }
    }

    pub(crate) fn stop(&mut self) {
        self.pause();
        self.player = None;
    }
}

impl Session {
    pub(crate) fn audio_preview_tick(&mut self) -> bool {
        let mut changed = false;
        let mut autoplay = None;
        let mut notice = None;
        for (pane, slot) in &mut self.previews.slots {
            let doc = match &slot.body {
                Body::Static(model)
                    if self.layout.states.contains_key(pane) && slot.playback_ready() =>
                {
                    match &**model {
                        Preview::Meta(meta) => meta.audio.as_ref(),
                        _ => None,
                    }
                }
                _ => None,
            };
            let Some(doc) = doc else {
                changed |= slot.audio.take().is_some();
                continue;
            };
            if slot.audio.as_ref().is_none_or(|audio| audio.doc != *doc) {
                slot.audio = Some(AudioState::new(doc.clone()));
                if self.prefs.preview_audio_autoplay {
                    autoplay = Some(*pane);
                }
                changed = true;
            }
            if let Some(audio) = &mut slot.audio
                && let Some(player) = &audio.player
            {
                match player.status() {
                    Ok(status) => {
                        changed |= (audio.position - status.position).abs() > 0.02;
                        audio.position = status.position;
                        if matches!(
                            status.state,
                            PlaybackState::Failed | PlaybackState::Finished
                        ) {
                            audio.playing = false;
                            audio.player = None;
                            if status.state == PlaybackState::Finished {
                                audio.position = 0.0;
                            }
                            notice = status.error.clone();
                            audio.error = status.error;
                            changed = true;
                        }
                    }
                    Err(err) => {
                        let error = err.to_string();
                        audio.error = Some(error.clone());
                        notice = Some(error);
                        audio.playing = false;
                        audio.player = None;
                        changed = true;
                    }
                }
            }
        }
        if let Some(pane) = autoplay {
            self.dispatch_or_notice(
                "preview.audio.play",
                &serde_json::json!({"pane": pane, "on": true}),
            );
        }
        if let Some(error) = notice {
            self.notice = Some(error);
        }
        changed
    }
}

fn state<'a>(s: &'a mut Session, params: &Params) -> Result<&'a mut AudioState, FmError> {
    let pane = s.preview_pane(params)?;
    s.previews
        .slots
        .get_mut(&pane)
        .and_then(|slot| slot.audio.as_mut())
        .ok_or_else(|| FmError::InvalidParams("the preview is not playable audio".into()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "preview.audio.play",
            label: "Play / Pause Audio",
            menu: &["View", "Preview", "Play / pause audio"],
            shortcut: None,
            params: "{on?: bool, pane?: number}: toggle or set playback",
            enabled: |s| s.previews.slots.values().any(|slot| slot.audio.is_some()),
            run: |s, params| {
                let on = object(params)?
                    .and_then(|map| map.get("on"))
                    .map(|v| {
                        v.as_bool()
                            .ok_or_else(|| FmError::InvalidParams("on: bool".into()))
                    })
                    .transpose()?;
                let pane = s.preview_pane(params)?;
                let on = on.unwrap_or(!state(s, params)?.playing);
                if on {
                    s.claim_audio(AudioOwner::PreviewAudio(pane));
                }
                let result = state(s, params)?.play(on);
                if !on || result.is_err() {
                    s.release_audio(AudioOwner::PreviewAudio(pane));
                }
                result
            },
        },
        CommandSpec {
            id: "preview.audio.seek",
            label: "Seek Audio",
            menu: &["View", "Preview", "Seek audio"],
            shortcut: None,
            params: "{to: number, pane?: number}: absolute position in seconds",
            enabled: |s| s.previews.slots.values().any(|slot| slot.audio.is_some()),
            run: |s, params| {
                let to = object(params)?
                    .and_then(|map| map.get("to"))
                    .and_then(Params::as_f64)
                    .filter(|to| to.is_finite() && *to >= 0.0)
                    .ok_or_else(|| {
                        FmError::InvalidParams("to: non-negative finite seconds".into())
                    })?;
                let audio = state(s, params)?;
                let to = audio
                    .doc
                    .duration
                    .map_or(to, |d| to.min((d - 0.01).max(0.0)));
                if let Some(player) = &audio.player {
                    player.seek(to)?;
                }
                audio.position = to;
                Ok(())
            },
        },
        CommandSpec {
            id: "preview.audio.volume",
            label: "Audio Volume",
            menu: &["View", "Preview", "Audio volume"],
            shortcut: None,
            params: "{value: number, pane?: number}: volume from 0 to 100",
            enabled: |s| s.previews.slots.values().any(|slot| slot.audio.is_some()),
            run: |s, params| {
                let value = object(params)?
                    .and_then(|map| map.get("value"))
                    .and_then(Params::as_f64)
                    .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                    .ok_or_else(|| FmError::InvalidParams("value: volume from 0 to 100".into()))?;
                let audio = state(s, params)?;
                audio.gain.set_volume(value as f32)?;
                audio.volume = value as f32;
                Ok(())
            },
        },
        CommandSpec {
            id: "preview.audio.mute",
            label: "Mute Audio",
            menu: &["View", "Preview", "Mute audio"],
            shortcut: None,
            params: "{on?: bool, pane?: number}: toggle or set mute",
            enabled: |s| s.previews.slots.values().any(|slot| slot.audio.is_some()),
            run: |s, params| {
                let on = object(params)?
                    .and_then(|map| map.get("on"))
                    .map(|v| {
                        v.as_bool()
                            .ok_or_else(|| FmError::InvalidParams("on: bool".into()))
                    })
                    .transpose()?;
                let audio = state(s, params)?;
                audio.muted = on.unwrap_or(!audio.muted);
                audio.gain.set_muted(audio.muted);
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use fm_layout::PaneId;
    use serde_json::json;
    use std::sync::Arc;

    fn setup() -> (Session, PaneId) {
        let mut s = Session::new().expect("session");
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("preview");
        let pane = s.layout.focused();
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.body = Body::Static(Arc::new(Preview::Meta(fm_preview::MetaDoc {
            rows: vec![],
            hint: None,
            poster: None,
            video: None,
            audio: Some(AudioDoc {
                path: "/missing/audio.wav".into(),
                duration: Some(60.0),
            }),
        })));
        s.previews.slots.insert(pane, slot);
        s.audio_preview_tick();
        (s, pane)
    }

    #[test]
    fn audio_starts_quiet_and_controls_validate_before_changing_state() {
        let (mut s, pane) = setup();
        assert!(!s.prefs.preview_audio_autoplay);
        assert!(
            !s.previews.slots[&pane]
                .audio
                .as_ref()
                .expect("audio")
                .playing
        );
        for (command, params) in [
            ("preview.audio.play", json!({"on": "yes"})),
            ("preview.audio.seek", json!({"to": -1})),
            ("preview.audio.seek", json!({"to": "1"})),
            ("preview.audio.volume", json!({"value": 101})),
            ("preview.audio.mute", json!({"on": 1})),
            (
                "prefs.set",
                json!({"key": "preview_audio_autoplay", "value": "yes"}),
            ),
        ] {
            assert!(s.dispatch(command, &params).is_err(), "{command}");
        }
        s.dispatch("preview.audio.volume", &json!({"value": 37}))
            .expect("volume");
        s.dispatch("preview.audio.mute", &json!({"on": true}))
            .expect("mute");
        s.dispatch("preview.audio.seek", &json!({"to": 12}))
            .expect("seek");
        let audio = s.previews.slots[&pane].audio.as_ref().expect("audio");
        assert_eq!(audio.volume, 37.0);
        assert!(audio.muted);
        assert_eq!(audio.position, 12.0);
        assert!(
            audio.player.is_none(),
            "adjusting controls must not open the device"
        );
    }

    #[test]
    fn changing_preview_content_removes_the_audio_player() {
        let (mut s, pane) = setup();
        s.previews.slots.get_mut(&pane).expect("slot").body = Body::Empty;
        s.audio_preview_tick();
        assert!(s.previews.slots[&pane].audio.is_none());
    }

    #[test]
    fn starting_video_pauses_audio_and_starting_audio_pauses_video() {
        let (mut s, audio_pane) = setup();
        let pane = s
            .layout
            .split(audio_pane, fm_layout::Axis::Horizontal, 0.5)
            .expect("split");
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.body = Body::Static(Arc::new(Preview::Meta(fm_preview::MetaDoc {
            rows: vec![],
            hint: None,
            poster: None,
            audio: None,
            video: Some(fm_preview::VideoInfo {
                path: "/missing/video.mp4".into(),
                range: None,
                width: 640,
                height: 360,
                fps: 25.0,
                duration: 10.0,
                audio: None,
                audio_delay: 0.0,
                tracks: Default::default(),
            }),
        })));
        s.previews.slots.insert(pane, slot);
        s.previews
            .slots
            .get_mut(&audio_pane)
            .expect("slot")
            .audio
            .as_mut()
            .expect("audio")
            .playing = true;
        s.video_tick();
        assert!(
            !s.previews.slots[&audio_pane]
                .audio
                .as_ref()
                .expect("audio")
                .playing
        );
        assert!(
            s.previews.slots[&pane]
                .video
                .as_ref()
                .expect("video")
                .playing
        );
        assert_eq!(s.audio_owner, AudioOwner::PreviewVideo(pane));
        s.dispatch(
            "preview.audio.play",
            &json!({"pane": audio_pane, "on": true}),
        )
        .expect("play");
        assert!(
            !s.previews.slots[&pane]
                .video
                .as_ref()
                .expect("video")
                .playing
        );
        assert!(
            s.previews.slots[&audio_pane]
                .audio
                .as_ref()
                .expect("audio")
                .playing
        );
        assert_eq!(s.audio_owner, AudioOwner::PreviewAudio(audio_pane));
    }

    #[test]
    fn closed_pane_drops_audio_state() {
        let (mut s, pane) = setup();
        s.claim_audio(AudioOwner::PreviewAudio(pane));
        s.layout
            .split(pane, fm_layout::Axis::Horizontal, 0.5)
            .expect("split");
        s.dispatch("pane.close", &json!({"pane": pane}))
            .expect("close");
        s.audio_preview_tick();
        s.reconcile_audio_owner();
        assert!(s.previews.slots[&pane].audio.is_none());
        assert_eq!(s.audio_owner, AudioOwner::None);
    }

    #[test]
    fn audio_commands_without_audio_fail_gracefully() {
        let mut s = Session::new().expect("session");
        for (id, params) in [
            ("preview.audio.play", json!({})),
            ("preview.audio.seek", json!({"to": 1})),
            ("preview.audio.volume", json!({"value": 50})),
            ("preview.audio.mute", json!({})),
        ] {
            assert!(s.dispatch(id, &params).is_err(), "{id}");
        }
    }
}
