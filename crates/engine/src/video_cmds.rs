//! `preview.video.*`: play a local video inside a Preview pane.
//!
//! `fm-preview::player` owns the ffmpeg child; this module owns the per-pane state
//! (playing, position, decode width) and the commands. The UI shows [`VideoState::frame`] and
//! sends `fit` (the width it can use), `play` and `seek`. At most one decoder runs per pane;
//! it is dropped (killed) when the pane shows something else or is closed.

use std::sync::Arc;

use fm_layout::PaneId;
use fm_preview::player::{Frame, MAX_WIDTH, MIN_WIDTH, PlaySpec, Player};
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
    player: Option<Player>,
    need_start: bool,
}

impl VideoState {
    fn new(info: VideoInfo) -> Self {
        Self {
            info,
            width: None,
            playing: true,
            pos: 0.0,
            frame: None,
            error: None,
            player: None,
            need_start: true,
        }
    }

    /// No decoder is running (paused, ended or not started).
    pub fn player_is_idle(&self) -> bool {
        self.player.is_none() && !self.need_start
    }

    fn restart(&mut self) {
        self.player = None;
        self.need_start = true;
    }

    fn duration(&self) -> f64 {
        self.info.duration.max(0.0)
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
        for (pane, slot) in self.previews.slots.iter_mut() {
            // A video in a background workspace stops (the decoder is killed).
            let active = self.layout.states.contains_key(pane);
            let info = if active {
                video_info(slot).cloned()
            } else {
                None
            };
            let Some(info) = info else {
                changed |= slot.video.take().is_some();
                continue;
            };
            if slot.video.as_ref().is_none_or(|v| v.info != info) {
                slot.video = Some(VideoState::new(info));
                changed = true;
            }
            if let Some(v) = &mut slot.video {
                changed |= pump(v);
            }
        }
        changed
    }
}

fn pump(v: &mut VideoState) -> bool {
    let mut changed = false;
    if v.need_start
        && let Some(width) = v.width
    {
        v.need_start = false;
        let spec = PlaySpec {
            info: v.info.clone(),
            from: v.pos,
            width,
            still: !v.playing,
        };
        match Player::start(&spec) {
            Some(p) => v.player = Some(p),
            None => {
                v.playing = false;
                v.error = Some("Playback needs ffmpeg and a local file".into());
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
        v.frame = Some(f);
        if v.playing {
            v.pos = player.position().min(v.duration().max(player.position()));
        }
        changed = true;
    }
    if player.finished() {
        if v.playing && !player.still {
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
            let (_, v) = video_slot(s, p)?;
            let want = on.unwrap_or(!v.playing);
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
    vec![play(), seek(), fit()]
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
        VideoState::new(VideoInfo {
            path: "/v.mp4".into(),
            width: 640,
            height: 360,
            fps: 25.0,
            duration: 10.0,
        })
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
    fn an_unavailable_decoder_pauses_with_a_message() {
        let mut v = state();
        v.info.path = "-i".into();
        v.width = Some(640);
        assert!(pump(&mut v));
        assert!(!v.playing);
        assert!(v.error.is_some());
    }
}
