//! `preview.video.*`: play a local video inside a Preview pane.
//!
//! `fm-preview::player` owns the ffmpeg child; this module owns the per-pane state
//! (playing, position, decode width) and the commands. The UI shows [`VideoState::frame`] and
//! sends `fit` (the width it can use), `play` and `seek`. At most one decoder runs per pane;
//! it is dropped (killed) when the pane shows something else or is closed.

use std::sync::Arc;

use crate::audio_owner::AudioOwner;
use fm_layout::PaneId;
use fm_media::Gain;
use fm_preview::player::{
    Decoding, Frame, HwAccel, MAX_WIDTH, MIN_WIDTH, Overlay, PlaySpec, Player,
};
use fm_preview::{Preview, VideoInfo};

use crate::file_cmds::object;
use crate::preview_cmds::{Body, PreviewSlot};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;
use fm_vfs::FmError;

/// Decode widths move in steps so resizing a pane does not restart ffmpeg on every pixel.
const WIDTH_STEP: u32 = 64;
/// Within this many seconds of the end, Play starts over.
const END_SLACK: f64 = 0.25;

/// Wait for decoder cleanup only after the UI event loop is leaving.
pub fn shutdown_players() -> Result<(), FmError> {
    fm_preview::player::shutdown()
}

#[derive(Debug)]
pub struct VideoState {
    pub info: VideoInfo,
    /// Decode width reported by the UI; nothing starts before it is known.
    pub width: Option<u32>,
    pub playing: bool,
    /// Seconds into the video.
    pub pos: f64,
    /// Newest decoded frame (kept while paused).
    pub frame: Option<Arc<Frame>>,
    /// Why playback is unavailable.
    pub error: Option<String>,
    /// Recoverable degradation, such as a missing audio device.
    pub hint: Option<String>,
    pub volume: f32,
    pub muted: bool,
    pub speed: f64,
    /// Hardware or software decoding, known once the running decoder showed a frame.
    pub decoding: Option<Decoding>,
    /// Try hardware decoding first (the preference, set by the owner every tick).
    hardware: bool,
    /// Picture subtitles drawn into the frames (Media pane only).
    overlay: Option<Overlay>,
    /// Why hardware decoding stopped mid-stream, for the message log.
    fallback: Option<String>,
    gain: Gain,
    player: Option<Player>,
    need_start: bool,
}

impl VideoState {
    pub(crate) fn new(info: VideoInfo, autoplay: bool) -> Self {
        Self {
            info,
            width: None,
            playing: autoplay,
            pos: 0.0,
            frame: None,
            error: None,
            hint: None,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            decoding: None,
            hardware: true,
            overlay: None,
            fallback: None,
            gain: Gain::default(),
            player: None,
            need_start: autoplay,
        }
    }

    pub(crate) fn with_gain(info: VideoInfo, autoplay: bool, gain: Gain) -> Self {
        let mut video = Self::new(info, autoplay);
        video.gain = gain;
        video
    }

    /// No decoder is running (paused, ended or not started).
    pub fn player_is_idle(&self) -> bool {
        self.player.is_none() && !self.need_start
    }

    pub(crate) fn restart(&mut self) {
        self.player = None;
        self.need_start = true;
    }

    pub(crate) fn stop(&mut self) {
        self.player = None;
        self.need_start = false;
    }

    pub(crate) fn pause(&mut self) {
        self.playing = false;
        self.stop();
    }

    pub(crate) fn seek(&mut self, to: f64) {
        self.pos = to;
        self.error = None;
        self.restart();
    }

    fn duration(&self) -> f64 {
        self.info.duration.max(0.0)
    }

    /// Follow the hardware decoding preference; a change restarts the decoder.
    pub(crate) fn set_hardware(&mut self, on: bool) {
        if self.hardware == on {
            return;
        }
        self.hardware = on;
        self.decoding = None;
        if self.player.is_some() || (self.frame.is_some() && !self.playing) {
            self.restart();
        }
    }

    /// Draw `overlay` (picture subtitles) from now on. While playing only the video decoder
    /// restarts, so audio continues; a paused video decodes a new still frame.
    pub(crate) fn set_overlay(&mut self, overlay: Option<Overlay>) {
        if self.overlay == overlay {
            return;
        }
        self.overlay = overlay;
        match &self.player {
            Some(player) if self.playing && !player.still => {
                player.set_overlay(overlay, &self.info);
            }
            Some(_) => self.restart(),
            None if !self.playing && !self.need_start && self.frame.is_some() => self.restart(),
            None => {}
        }
    }

    /// Why hardware decoding stopped during playback; returned once.
    pub(crate) fn take_fallback(&mut self) -> Option<String> {
        self.fallback.take()
    }
}

/// The decode width the engine will use for a pane `pixels` wide.
pub fn width_bucket(pixels: f32) -> u32 {
    let step = f64::from(WIDTH_STEP);
    let px = f64::from(pixels).max(1.0);
    ((px / step).ceil() * step).clamp(f64::from(MIN_WIDTH), f64::from(MAX_WIDTH)) as u32
}

fn video_info(slot: &PreviewSlot) -> Option<&VideoInfo> {
    match &slot.body {
        Body::Static(p) => match &**p {
            Preview::Meta(m) => m.video.as_ref(),
            _ => None,
        },
        _ => None,
    }
}

impl Session {
    /// Create, start, feed and finish the per-pane players; true when the UI should repaint.
    pub(crate) fn video_tick(&mut self) -> bool {
        let mut changed = false;
        let mut started = None;
        // The OS path of a file just opened in another player (see `pause_previews_of`).
        let held = self
            .opened_elsewhere
            .as_ref()
            .filter(|(uri, _)| !self.preview_may_autoplay(Some(uri)))
            .and_then(|(uri, _)| uri.local_path_string());
        for (pane, slot) in self.previews.slots.iter_mut() {
            // A video in a background workspace stops (the decoder is killed).
            let active = self.layout.states.contains_key(pane);
            let info = if active && slot.playback_ready() {
                video_info(slot).cloned()
            } else {
                None
            };
            let Some(info) = info else {
                changed |= slot.video.take().is_some();
                continue;
            };
            if slot.video.as_ref().is_none_or(|v| v.info != info) {
                // A refreshed probe of the same video keeps it paused if it was paused.
                let was_paused = slot
                    .video
                    .as_ref()
                    .is_some_and(|v| !v.playing && v.info.path == info.path);
                let opened = held.as_deref() == Some(info.path.as_str());
                let autoplay = self.prefs.preview_video_autoplay && !was_paused && !opened;
                slot.video = Some(VideoState::new(info, autoplay));
                if autoplay {
                    started = Some(*pane);
                }
                changed = true;
            }
        }
        if let Some(pane) = started {
            self.claim_audio(AudioOwner::PreviewVideo(pane));
        }
        let hardware = self.prefs.media_hardware_decoding;
        for slot in self.previews.slots.values_mut() {
            if let Some(v) = &mut slot.video {
                v.set_hardware(hardware);
                changed |= pump(v);
                if let Some(reason) = v.take_fallback() {
                    self.messages.push(&reason);
                }
            }
        }
        changed
    }
}

pub(crate) fn pump(v: &mut VideoState) -> bool {
    let mut changed = false;
    if v.need_start
        && let Some(width) = v.width
    {
        v.need_start = false;
        let spec = PlaySpec {
            info: v.info.clone(),
            from: v.pos,
            width,
            speed: v.speed,
            still: !v.playing,
            hardware: v.hardware.then(HwAccel::for_platform),
            overlay: v.overlay,
        };
        match Player::start(&spec, v.gain.clone()) {
            Ok(p) => {
                crate::diag::count(crate::diag::Counter::DecoderStarts);
                crate::diag::event("decoder", || {
                    format!(
                        "start width={} source={}x{} from={:.1}s still={} hardware={} subtitles={} speed={}",
                        spec.width,
                        spec.info.width,
                        spec.info.height,
                        spec.from,
                        spec.still,
                        spec.hardware.map_or("off", |h| h.label()),
                        spec.overlay.is_some(),
                        spec.speed,
                    )
                });
                v.player = Some(p);
            }
            Err(err) => {
                crate::diag::event("decoder", || "start failed".into());
                v.playing = false;
                v.error = Some(err.to_string());
                changed = true;
            }
        }
    }
    let Some(player) = &v.player else {
        return changed;
    };
    if let Some(f) = player.latest()
        && !v.frame.as_ref().is_some_and(|old| Arc::ptr_eq(old, &f))
    {
        crate::diag::count(crate::diag::Counter::VideoFrames);
        v.frame = Some(f);
        changed = true;
    }
    if v.playing {
        let position = player.position();
        let position = if v.duration() > 0.0 {
            position.min(v.duration())
        } else {
            position
        };
        changed |= (v.pos - position).abs() > 0.02;
        v.pos = position;
    }
    let decoding = player.decoding();
    if decoding.is_some() && v.decoding != decoding {
        v.decoding = decoding;
        changed = true;
    }
    if let Some(reason) = player.take_fallback() {
        v.fallback = Some(reason);
        changed = true;
    }
    let hint = player.hint();
    if v.hint != hint {
        v.hint = hint;
        changed = true;
    }
    if player.finished() {
        if let Some(err) = player.error() {
            v.error = Some(err);
            v.playing = false;
        } else if v.playing && !player.still {
            v.playing = false;
            v.pos = v.duration();
        }
        v.player = None;
        changed = true;
    }
    changed
}

fn video_slot<'a>(
    s: &'a mut Session,
    params: &Params,
) -> Result<(PaneId, &'a mut VideoState), FmError> {
    let pane = s.preview_pane(params)?;
    s.previews
        .slots
        .get_mut(&pane)
        .and_then(|slot| slot.video.as_mut())
        .map(|v| (pane, v))
        .ok_or_else(|| FmError::InvalidParams("the preview is not a playable video".into()))
}

fn has_video(s: &Session) -> bool {
    s.previews.slots.values().any(|slot| slot.video.is_some())
}

fn number(params: &Params, key: &str) -> Result<Option<f64>, FmError> {
    object(params)?
        .and_then(|m| m.get(key))
        .map(|v| {
            v.as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| FmError::InvalidParams(format!("{key}: finite number")))
        })
        .transpose()
}

/// `preview.video.play {on?: bool, pane?: number}` — play or pause (toggles without `on`).
pub fn play() -> CommandSpec {
    CommandSpec {
        id: "preview.video.play",
        label: "Play / Pause Video",
        menu: &["View", "Preview", "Play / pause video"],
        shortcut: None,
        params: "{on?: bool, pane?: number}",
        enabled: has_video,
        run: |s, p: &Params| {
            let on = object(p)?
                .and_then(|m| m.get("on"))
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| FmError::InvalidParams("on: bool".into()))
                })
                .transpose()?;
            let want = on.unwrap_or(!video_slot(s, p)?.1.playing);
            let pane = video_slot(s, p)?.0;
            if want {
                s.claim_audio(AudioOwner::PreviewVideo(pane));
            }
            let (_, v) = video_slot(s, p)?;
            if want == v.playing {
                return Ok(());
            }
            v.playing = want;
            v.error = None;
            if want {
                if v.pos >= v.duration() - END_SLACK {
                    v.pos = 0.0;
                }
                v.restart();
            } else {
                v.player = None;
                v.need_start = false;
                s.release_audio(AudioOwner::PreviewVideo(pane));
            }
            Ok(())
        },
    }
}

/// `preview.video.seek {to: number, pane?: number}` — jump to `to` seconds.
pub fn seek() -> CommandSpec {
    CommandSpec {
        id: "preview.video.seek",
        label: "Seek Video",
        menu: &["View", "Preview", "Seek video"],
        shortcut: None,
        params: "{to: number, pane?: number}",
        enabled: has_video,
        run: |s, p: &Params| {
            let to = number(p, "to")?
                .ok_or_else(|| FmError::InvalidParams("to: number of seconds".into()))?;
            let (_, v) = video_slot(s, p)?;
            let max = (v.duration() - 0.05).max(0.0);
            v.pos = to.clamp(0.0, max);
            v.error = None;
            v.restart();
            Ok(())
        },
    }
}

/// `preview.video.fit {width: number, pane?: number}` — the width in pixels the pane can show;
/// decoding happens at this width (rounded up to a step, clamped, never above the source).
pub fn fit() -> CommandSpec {
    CommandSpec {
        id: "preview.video.fit",
        label: "Fit Video Decode Size",
        menu: &["View", "Preview", "Fit video size"],
        shortcut: None,
        params: "{width: number, pane?: number}",
        enabled: has_video,
        run: |s, p: &Params| {
            let width = number(p, "width")?
                .filter(|w| *w >= 1.0)
                .ok_or_else(|| FmError::InvalidParams("width: positive number".into()))?;
            let (_, v) = video_slot(s, p)?;
            let bucket = width_bucket(width as f32);
            if v.width != Some(bucket) {
                v.width = Some(bucket);
                if v.playing || v.player.is_some() {
                    v.restart();
                }
            }
            Ok(())
        },
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![play(), seek(), fit(), volume(), mute()]
}

fn volume() -> CommandSpec {
    CommandSpec {
        id: "preview.video.volume",
        label: "Video Volume",
        menu: &["View", "Preview", "Video volume"],
        shortcut: None,
        params: "{value: number, pane?: number}: volume from 0 to 100",
        enabled: has_video,
        run: |s, params| {
            let value = number(params, "value")?
                .filter(|v| (0.0..=100.0).contains(v))
                .ok_or_else(|| FmError::InvalidParams("value: volume from 0 to 100".into()))?;
            let (_, video) = video_slot(s, params)?;
            video.gain.set_volume(value as f32)?;
            video.volume = value as f32;
            Ok(())
        },
    }
}

fn mute() -> CommandSpec {
    CommandSpec {
        id: "preview.video.mute",
        label: "Mute Video",
        menu: &["View", "Preview", "Mute video"],
        shortcut: None,
        params: "{on?: bool, pane?: number}: toggle or set mute",
        enabled: has_video,
        run: |s, params| {
            let on = object(params)?
                .and_then(|map| map.get("on"))
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| FmError::InvalidParams("on: bool".into()))
                })
                .transpose()?;
            let (_, video) = video_slot(s, params)?;
            video.muted = on.unwrap_or(!video.muted);
            video.gain.set_muted(video.muted);
            Ok(())
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use serde_json::json;

    use super::*;

    #[test]
    fn commands_without_a_video_fail_gracefully() {
        let mut s = Session::new().expect("session");
        for (id, params) in [
            ("preview.video.play", json!({})),
            ("preview.video.seek", json!({ "to": 3 })),
            ("preview.video.fit", json!({ "width": 640 })),
            ("preview.video.seek", json!({ "to": "x" })),
            ("preview.video.fit", json!({ "width": -1 })),
        ] {
            assert!(s.dispatch(id, &params).is_err(), "{id} {params}");
        }
    }

    fn state() -> VideoState {
        VideoState::new(
            VideoInfo {
                path: "/v.mp4".into(),
                range: None,
                width: 640,
                height: 360,
                fps: 25.0,
                duration: 10.0,
                audio: None,
                audio_delay: 0.0,
                tracks: Default::default(),
            },
            true,
        )
    }

    #[test]
    fn the_hardware_switch_and_subtitle_overlay_restart_only_when_they_change() {
        let mut v = state();
        v.need_start = false;
        v.playing = false;
        // Nothing decoded yet: nothing to redo.
        v.set_hardware(true);
        v.set_overlay(None);
        assert!(v.player_is_idle());
        v.set_hardware(false);
        assert!(v.player_is_idle(), "no frame yet, no decoder needed");
        // A paused video with a still frame decodes it again with the new settings.
        v.frame = Some(Arc::new(Frame::new(2, 2, vec![0; 16], 0)));
        v.set_hardware(true);
        assert!(v.need_start, "switching decoding redoes the still frame");
        v.need_start = false;
        let overlay = Overlay {
            stream: 0,
            scale: 1.0,
            lift: 0.0,
        };
        v.set_overlay(Some(overlay));
        assert!(v.need_start, "new subtitles redo the still frame");
        v.need_start = false;
        v.set_overlay(Some(overlay));
        v.set_hardware(true);
        assert!(!v.need_start, "unchanged settings leave the decoder alone");
        assert_eq!(v.take_fallback(), None);
    }

    #[test]
    fn a_new_state_autoplays_once_the_width_is_known() {
        let mut v = state();
        assert!(v.playing && v.need_start && v.width.is_none());
        // Without a width nothing starts, and nothing is reported as changed.
        assert!(!pump(&mut v));
        assert!(v.need_start);
    }

    #[test]
    fn disabling_video_autoplay_leaves_the_poster_without_starting_a_decoder() {
        let mut s = Session::new().expect("session");
        s.prefs.preview_video_autoplay = false;
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("kind");
        let pane = s.layout.focused();
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(Arc::new(Preview::Meta(fm_preview::MetaDoc {
            rows: vec![],
            hint: None,
            poster: None,
            audio: None,
            video: Some(state().info),
        })));
        s.previews.slots.insert(pane, slot);
        s.video_tick();
        let video = s.previews.slots[&pane].video.as_ref().expect("video");
        assert!(!video.playing && video.player_is_idle());
        assert!(
            s.dispatch(
                "prefs.set",
                &json!({"key": "preview_video_autoplay", "value": "yes"})
            )
            .is_err()
        );
    }

    #[test]
    fn a_video_opened_elsewhere_while_its_preview_loads_does_not_autoplay() {
        let mut s = Session::new().expect("session");
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("kind");
        let pane = s.layout.focused();
        let uri = fm_vfs::VfsUri::parse("/v.mp4").expect("uri");
        let mut info = state().info;
        info.path = uri.local_path_string().expect("local");
        let show = |s: &mut Session, info: &VideoInfo| {
            let slot = s
                .previews
                .slots
                .entry(pane)
                .or_insert_with(PreviewSlot::new);
            slot.body = Body::Static(Arc::new(Preview::Meta(fm_preview::MetaDoc {
                rows: vec![],
                hint: None,
                poster: None,
                audio: None,
                video: Some(info.clone()),
            })));
            s.video_tick();
            s.previews.slots[&pane]
                .video
                .as_ref()
                .expect("video")
                .playing
        };
        // The double-click opens the file before the first click's preview has loaded.
        s.pause_previews_of(&uri);
        assert!(!show(&mut s, &info), "loaded after the open: stays paused");
        info.duration = 11.0;
        assert!(!show(&mut s, &info), "a refreshed probe keeps it paused");
        s.opened_elsewhere = None;
        let mut other = info.clone();
        other.path.push_str(".other");
        assert!(show(&mut s, &other), "another video still autoplays");
    }

    #[test]
    fn an_unavailable_decoder_pauses_with_a_message() {
        let mut v = state();
        v.info.path = "-i".into();
        v.width = Some(640);
        assert!(pump(&mut v));
        assert!(!v.playing);
        assert!(v.error.is_some());
    }

    fn insert_video(s: &mut Session, pane: PaneId) {
        let mut slot = PreviewSlot::new();
        slot.video = Some(state());
        s.previews.slots.insert(pane, slot);
    }

    #[test]
    fn volume_mute_and_invalid_commands_do_not_restart_the_decoder() {
        let mut s = Session::new().expect("session");
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("kind");
        let pane = s.layout.focused();
        insert_video(&mut s, pane);
        for (command, params) in [
            ("preview.video.volume", json!({"value": -1})),
            ("preview.video.volume", json!({"value": 101})),
            ("preview.video.volume", json!({"value": "loud"})),
            ("preview.video.mute", json!({"on": 1})),
        ] {
            assert!(s.dispatch(command, &params).is_err(), "{command}");
        }
        s.dispatch("preview.video.volume", &json!({"value": 37}))
            .expect("volume");
        s.dispatch("preview.video.mute", &json!({"on": true}))
            .expect("mute");
        let video = s.previews.slots[&pane].video.as_ref().expect("video");
        assert_eq!(video.volume, 37.0);
        assert!(video.muted);
        assert!(
            video.player.is_none(),
            "volume/mute must not start a decoder"
        );
    }

    #[test]
    fn starting_a_second_preview_video_pauses_the_first() {
        let mut s = Session::new().expect("session");
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("kind");
        let first = s.layout.focused();
        let second = s
            .layout
            .split(first, fm_layout::Axis::Horizontal, 0.5)
            .expect("split");
        insert_video(&mut s, first);
        insert_video(&mut s, second);
        s.dispatch("preview.video.play", &json!({"pane": second, "on": true}))
            .expect("play");
        let old = s.previews.slots[&first].video.as_ref().expect("video");
        assert!(!old.playing && !old.need_start && old.player.is_none());
        assert!(
            s.previews.slots[&second]
                .video
                .as_ref()
                .expect("video")
                .playing
        );
    }
}
