//! Audio extras of the Media pane: gapless hand-over, the ten-band equalizer, the A-B loop
//! and the sleep timer. All of it is cheap bookkeeping on the UI tick; decoding, the filter
//! and the fade run on the audio threads.

use std::time::{Duration, Instant};

use fm_media::{BANDS, EqSettings, PRESETS};
use fm_vfs::{FmError, VfsUri};
use serde_json::Value;

use crate::Session;
use crate::file_cmds::object;
use crate::media_cmds::is_video;
use crate::registry::CommandSpec;

pub const MAX_USER_PRESETS: usize = 32;
pub const MAX_PRESET_NAME: usize = 40;
pub const MAX_SLEEP_MINUTES: u64 = 720;
const FADE_SECS: f64 = 10.0;
const MIN_LOOP_SECS: f64 = 0.5;
const MAX_ARMED: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct UserPreset {
    pub name: String,
    pub bands: [f32; BANDS],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sleep {
    At(Instant),
    EndOfItem,
}

#[derive(Debug, Default)]
pub struct AudioExtras {
    /// Name of the preset the bands came from; cleared by hand edits.
    pub eq_preset: Option<String>,
    pub user_presets: Vec<UserPreset>,
    pub loop_a: Option<f64>,
    pub loop_b: Option<f64>,
    pub sleep: Option<Sleep>,
    armed: Vec<(u64, VfsUri)>,
    armed_uri: Option<VfsUri>,
    last_token: u64,
    seen_token: u64,
}

impl AudioExtras {
    /// The player was replaced or the item changed: nothing is armed any more.
    pub(crate) fn item_changed(&mut self) {
        self.armed.clear();
        self.armed_uri = None;
        self.seen_token = 0;
        self.loop_a = None;
        self.loop_b = None;
    }

    pub(crate) fn seen_token(&self) -> u64 {
        self.seen_token
    }

    pub fn sleep_remaining(&self) -> Option<Duration> {
        match self.sleep {
            Some(Sleep::At(at)) => Some(at.saturating_duration_since(Instant::now())),
            _ => None,
        }
    }
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

impl Session {
    /// Keep the player's "next item" in step with the queue so consecutive tracks join without
    /// a gap. Only local audio at normal speed is armed; everything else falls back to the
    /// ordinary stop-and-start advance.
    pub(crate) fn media_arm_gapless(&mut self) {
        let want = self.prefs.media_gapless
            && self.media.video.is_none()
            && !self.media.is_video()
            && self.media.extras.sleep != Some(Sleep::EndOfItem);
        let target = if want {
            self.media
                .queue
                .peek_next()
                .filter(|uri| !is_video(uri))
                .and_then(|uri| self.media.playable(uri).map(|path| (uri.clone(), path)))
                .and_then(|(uri, path)| path.local_path().map(|local| (uri, local)))
        } else {
            None
        };
        let Some(player) = &self.media.player else {
            return;
        };
        let extras = &mut self.media.extras;
        match target {
            Some((uri, _)) if extras.armed_uri.as_ref() == Some(&uri) => {}
            Some((uri, path)) => {
                extras.last_token += 1;
                let token = extras.last_token;
                if player.queue_next(path, token).is_ok() {
                    if extras.armed.len() >= MAX_ARMED {
                        extras.armed.remove(0);
                    }
                    extras.armed.push((token, uri.clone()));
                    extras.armed_uri = Some(uri);
                }
            }
            None if extras.armed_uri.is_some() => {
                player.clear_next();
                extras.armed_uri = None;
            }
            None => {}
        }
    }

    /// The player moved on to the armed item by itself: follow with the queue and drop the
    /// old item's state.
    pub(crate) fn media_gapless_advance(&mut self, token: u64) {
        let extras = &mut self.media.extras;
        extras.seen_token = token;
        let uri = extras
            .armed
            .iter()
            .find(|(armed, _)| *armed == token)
            .map(|(_, uri)| uri.clone());
        extras.armed.retain(|(armed, _)| *armed > token);
        extras.armed_uri = None;
        extras.loop_a = None;
        extras.loop_b = None;
        let advanced = self.media.queue.next_item().cloned();
        if uri.is_none() || advanced != uri {
            // The queue changed under us; restart cleanly from where it points now.
            self.media.reset_item();
            if let Err(error) = self.media_resume() {
                self.media.error = Some(error.to_string());
            }
            return;
        }
        self.media.info = None;
        if let Some((_, job)) = self.media.cover_job.take() {
            self.jobs.cancel(job);
        }
        self.media.cover = None;
        self.media.cover_file = None;
        self.media.error = None;
        self.media.failed_items = 0;
        let current = self.media.queue.current().cloned();
        self.media.spool.cancel_except(current.as_ref());
    }

    /// Loop and sleep-timer bookkeeping, once per tick.
    pub(crate) fn media_audio_tick(&mut self) -> bool {
        self.media_arm_gapless();
        let mut changed = false;
        if self.media.playing
            && let (Some(a), Some(b)) = (self.media.extras.loop_a, self.media.extras.loop_b)
            && self.media.position >= b
        {
            self.media_seek_raw(a);
            changed = true;
        }
        let Some(sleep) = self.media.extras.sleep else {
            return changed;
        };
        let remaining = match sleep {
            Sleep::At(at) => Some(at.saturating_duration_since(Instant::now()).as_secs_f64()),
            Sleep::EndOfItem => self
                .media
                .duration()
                .map(|duration| (duration - self.media.position).max(0.0)),
        };
        if !self.media.playing {
            // A paused timer is cancelled rather than silently stopping later playback.
            if matches!(sleep, Sleep::At(at) if at <= Instant::now()) {
                self.media_sleep_clear();
                changed = true;
            }
            return changed;
        }
        match remaining {
            Some(left) if matches!(sleep, Sleep::At(_)) && left <= 0.0 => {
                self.media.pause();
                self.media_sleep_clear();
                self.release_audio(crate::audio_owner::AudioOwner::Media);
                changed = true;
            }
            Some(left) if left < FADE_SECS => {
                self.media.gain.set_fade((left / FADE_SECS) as f32);
            }
            _ => {
                if self.media.gain.fade() < 1.0 {
                    self.media.gain.set_fade(1.0);
                }
            }
        }
        changed
    }

    pub(crate) fn media_sleep_clear(&mut self) {
        self.media.extras.sleep = None;
        self.media.gain.set_fade(1.0);
    }

    pub(crate) fn media_seek_raw(&mut self, to: f64) {
        if let Some(player) = &self.media.player {
            let _ = player.seek(to);
        }
        if let Some(video) = &mut self.media.video {
            video.seek(to);
        }
        self.media.position = to;
    }

    /// Apply the equalizer and its automatic preamp in one step.
    pub(crate) fn media_apply_eq(&mut self, settings: &EqSettings) -> Result<(), FmError> {
        settings.validate()?;
        self.media.gain.set_eq(settings)?;
        let preamp = if settings.enabled {
            settings.safe_preamp()
        } else {
            0.0
        };
        self.media.gain.set_preamp(preamp)
    }

    pub(crate) fn media_restore_eq(
        &mut self,
        settings: EqSettings,
        preset: Option<String>,
        user: Vec<UserPreset>,
    ) -> Result<(), FmError> {
        self.media_apply_eq(&settings)?;
        self.media.extras.eq_preset = preset;
        self.media.extras.user_presets = user;
        Ok(())
    }
}

fn clean_name(name: &str) -> Result<String, FmError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_PRESET_NAME
        || name.chars().any(char::is_control)
    {
        return Err(bad("preset names must be 1-40 printable characters"));
    }
    if PRESETS
        .iter()
        .any(|(builtin, _)| builtin.eq_ignore_ascii_case(name))
    {
        return Err(bad("that name belongs to a built-in preset"));
    }
    Ok(name.to_string())
}

fn eq_run(s: &mut Session, params: &crate::registry::Params) -> Result<(), FmError> {
    let map = object(params)?.ok_or_else(|| bad("equalizer parameters required"))?;
    if map.is_empty()
        || map
            .keys()
            .any(|k| !["enabled", "preset", "bands", "save_as", "delete"].contains(&k.as_str()))
    {
        return Err(bad("expected enabled, preset, bands, save_as or delete"));
    }
    let mut settings = s.media.gain.eq();
    let mut preset = s.media.extras.eq_preset.clone();
    let mut user = s.media.extras.user_presets.clone();
    if let Some(value) = map.get("enabled") {
        settings.enabled = value
            .as_bool()
            .ok_or_else(|| bad("enabled must be true or false"))?;
    }
    if let Some(value) = map.get("preset") {
        let name = value.as_str().ok_or_else(|| bad("preset must be a name"))?;
        let bands = PRESETS
            .iter()
            .find(|(builtin, _)| builtin.eq_ignore_ascii_case(name))
            .map(|(_, bands)| *bands)
            .or_else(|| user.iter().find(|p| p.name == name).map(|p| p.bands))
            .ok_or_else(|| bad("unknown equalizer preset"))?;
        settings.bands = bands;
        settings.enabled = map.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        preset = Some(name.to_string());
    }
    if let Some(value) = map.get("bands") {
        let list = value
            .as_array()
            .filter(|list| list.len() == BANDS)
            .ok_or_else(|| bad("bands must be a list of 10 numbers"))?;
        let mut bands = [0.0_f32; BANDS];
        for (slot, value) in bands.iter_mut().zip(list) {
            *slot = value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| bad("bands must be finite numbers"))? as f32;
        }
        settings.bands = bands;
        preset = None;
    }
    if let Some(value) = map.get("delete") {
        let name = value.as_str().ok_or_else(|| bad("delete must be a name"))?;
        let before = user.len();
        user.retain(|p| p.name != name);
        if user.len() == before {
            return Err(bad("unknown user preset"));
        }
        if preset.as_deref() == Some(name) {
            preset = None;
        }
    }
    settings.validate()?;
    if let Some(value) = map.get("save_as") {
        let name = clean_name(
            value
                .as_str()
                .ok_or_else(|| bad("save_as must be a name"))?,
        )?;
        let entry = UserPreset {
            name: name.clone(),
            bands: settings.bands,
        };
        if let Some(existing) = user.iter_mut().find(|p| p.name == name) {
            *existing = entry;
        } else if user.len() >= MAX_USER_PRESETS {
            return Err(bad("too many saved presets; delete one first"));
        } else {
            user.push(entry);
        }
        preset = Some(name);
    }
    s.media_apply_eq(&settings)?;
    s.media.extras.eq_preset = preset;
    s.media.extras.user_presets = user;
    Ok(())
}

fn loop_run(s: &mut Session, params: &crate::registry::Params) -> Result<(), FmError> {
    let map = object(params)?.ok_or_else(|| bad("loop parameters required"))?;
    if s.media.queue.current().is_none() {
        return Err(bad("no current media item"));
    }
    let seconds = |v: &Value| {
        v.as_f64()
            .filter(|v| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| bad("loop points must be non-negative seconds"))
    };
    let (mut a, mut b) = (s.media.extras.loop_a, s.media.extras.loop_b);
    match (map.get("point"), map.len()) {
        (Some(point), 1) => match point.as_str() {
            Some("a") => {
                a = Some(s.media.position);
                if b.is_some_and(|b| b < s.media.position + MIN_LOOP_SECS) {
                    b = None;
                }
            }
            Some("b") => {
                if a.is_none() {
                    return Err(bad("set point a first"));
                }
                b = Some(s.media.position);
            }
            Some("clear") => (a, b) = (None, None),
            _ => return Err(bad("point must be a, b or clear")),
        },
        (None, _) if !map.is_empty() => {
            if map.keys().any(|k| k != "a" && k != "b") {
                return Err(bad("expected point, or a and b"));
            }
            for (key, slot) in [("a", &mut a), ("b", &mut b)] {
                match map.get(key) {
                    Some(Value::Null) => *slot = None,
                    Some(v) => *slot = Some(seconds(v)?),
                    None => {}
                }
            }
        }
        _ => return Err(bad("expected point, or a and b")),
    }
    if let (Some(a), Some(b)) = (a, b)
        && b < a + MIN_LOOP_SECS
    {
        return Err(bad("point b must be at least half a second after a"));
    }
    if b.is_some() && a.is_none() {
        return Err(bad("set point a first"));
    }
    if let Some(duration) = s.media.duration()
        && [a, b].into_iter().flatten().any(|p| p > duration)
    {
        return Err(bad("loop points must be inside the item"));
    }
    s.media.extras.loop_a = a;
    s.media.extras.loop_b = b;
    Ok(())
}

fn sleep_run(s: &mut Session, params: &crate::registry::Params) -> Result<(), FmError> {
    let map = object(params)?.ok_or_else(|| bad("sleep parameters required"))?;
    if map.len() != 1 {
        return Err(bad("supply exactly one of minutes, end_of_item or cancel"));
    }
    if let Some(value) = map.get("minutes") {
        let minutes = value
            .as_u64()
            .filter(|m| (1..=MAX_SLEEP_MINUTES).contains(m))
            .ok_or_else(|| bad("minutes must be 1-720"))?;
        s.media.extras.sleep = Some(Sleep::At(
            Instant::now() + Duration::from_secs(minutes * 60),
        ));
    } else if let Some(value) = map.get("end_of_item") {
        if value.as_bool() != Some(true) {
            return Err(bad("end_of_item must be true"));
        }
        s.media.extras.sleep = Some(Sleep::EndOfItem);
    } else if let Some(value) = map.get("cancel") {
        if value.as_bool() != Some(true) {
            return Err(bad("cancel must be true"));
        }
        s.media_sleep_clear();
    } else {
        return Err(bad("supply exactly one of minutes, end_of_item or cancel"));
    }
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.eq",
            label: "Equalizer",
            menu: &["Media", "Equalizer"],
            shortcut: None,
            params: "at least one of {enabled: bool, preset: string, bands: number[10] (±12 dB), save_as: string, delete: string}; a preset turns the equalizer on and the preamp follows the largest boost",
            enabled: |_| true,
            run: eq_run,
        },
        CommandSpec {
            id: "media.loop",
            label: "A-B Loop",
            menu: &["Media", "A-B loop"],
            shortcut: None,
            params: "{point: \"a\"|\"b\"|\"clear\"} marks the current position, or {a?: seconds|null, b?: seconds|null}",
            enabled: |s| s.media.queue.current().is_some(),
            run: loop_run,
        },
        CommandSpec {
            id: "media.sleep",
            label: "Sleep Timer",
            menu: &["Media", "Sleep timer"],
            shortcut: None,
            params: "{minutes: 1-720} or {end_of_item: true} or {cancel: true}: fades out over 10 s, then pauses",
            enabled: |_| true,
            run: sleep_run,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn session() -> Session {
        Session::new().expect("session")
    }

    #[test]
    fn preset_and_bands_apply_and_set_a_safe_preamp() {
        let mut s = session();
        s.dispatch("media.eq", &json!({"preset": "Bass boost"}))
            .expect("preset");
        let eq = s.media.gain.eq();
        assert!(eq.enabled);
        assert_eq!(eq.bands[0], 6.0);
        assert_eq!(s.media.extras.eq_preset.as_deref(), Some("Bass boost"));
        s.dispatch(
            "media.eq",
            &json!({"bands": [0, 0, 0, 0, 0, 0, 0, 0, 0, 3]}),
        )
        .expect("bands");
        assert_eq!(s.media.extras.eq_preset, None);
        s.dispatch("media.eq", &json!({"enabled": false}))
            .expect("off");
        assert!(!s.media.gain.eq().enabled);
    }

    #[test]
    fn user_presets_save_replace_and_delete() {
        let mut s = session();
        s.dispatch(
            "media.eq",
            &json!({"bands": [1, 2, 3, 0, 0, 0, 0, 0, 0, 0], "save_as": "Mine"}),
        )
        .expect("save");
        s.dispatch("media.eq", &json!({"preset": "Flat"}))
            .expect("flat");
        s.dispatch("media.eq", &json!({"preset": "Mine"}))
            .expect("load");
        assert_eq!(s.media.gain.eq().bands[2], 3.0);
        s.dispatch("media.eq", &json!({"delete": "Mine"}))
            .expect("delete");
        assert!(s.media.extras.user_presets.is_empty());
        assert!(s.dispatch("media.eq", &json!({"delete": "Mine"})).is_err());
    }

    #[test]
    fn bad_equalizer_params_leave_the_settings_unchanged() {
        let mut s = session();
        s.dispatch("media.eq", &json!({"preset": "Rock"}))
            .expect("rock");
        let before = s.media.gain.eq();
        for params in [
            json!({}),
            json!(null),
            json!({"bands": [0, 0]}),
            json!({"bands": [13, 0, 0, 0, 0, 0, 0, 0, 0, 0]}),
            json!({"bands": ["x", 0, 0, 0, 0, 0, 0, 0, 0, 0]}),
            json!({"preset": "Nope"}),
            json!({"enabled": "yes"}),
            json!({"save_as": "Flat"}),
            json!({"save_as": "bad\nname"}),
            json!({"extra": 1}),
        ] {
            assert!(s.dispatch("media.eq", &params).is_err(), "{params}");
            assert_eq!(s.media.gain.eq(), before, "{params}");
        }
    }

    #[test]
    fn user_presets_are_capped() {
        let mut s = session();
        for n in 0..MAX_USER_PRESETS {
            s.dispatch("media.eq", &json!({"save_as": format!("p{n}")}))
                .expect("save");
        }
        assert!(
            s.dispatch("media.eq", &json!({"save_as": "one more"}))
                .is_err()
        );
    }

    #[test]
    fn loop_points_validate() {
        let mut s = session();
        s.media
            .queue
            .enqueue(VfsUri::parse("/a.mp3").expect("uri"))
            .expect("enqueue");
        s.media.queue.select(0).expect("select");
        s.media.position = 5.0;
        assert!(s.dispatch("media.loop", &json!({"point": "b"})).is_err());
        s.dispatch("media.loop", &json!({"point": "a"})).expect("a");
        s.media.position = 5.2;
        assert!(s.dispatch("media.loop", &json!({"point": "b"})).is_err());
        s.media.position = 9.0;
        s.dispatch("media.loop", &json!({"point": "b"})).expect("b");
        assert_eq!(
            (s.media.extras.loop_a, s.media.extras.loop_b),
            (Some(5.0), Some(9.0))
        );
        for params in [
            json!({"point": "c"}),
            json!({"a": -1}),
            json!({"a": 4, "b": 3}),
            json!({"a": "x"}),
            json!({"point": "a", "b": 1}),
            json!(7),
        ] {
            assert!(s.dispatch("media.loop", &params).is_err(), "{params}");
        }
        s.dispatch("media.loop", &json!({"point": "clear"}))
            .expect("clear");
        assert_eq!(s.media.extras.loop_a, None);
    }

    #[test]
    fn loop_returns_to_a_when_b_is_reached() {
        let mut s = session();
        s.media.extras.loop_a = Some(2.0);
        s.media.extras.loop_b = Some(4.0);
        s.media.playing = true;
        s.media.position = 4.1;
        assert!(s.media_audio_tick());
        assert_eq!(s.media.position, 2.0);
        s.media.position = 3.0;
        s.media_audio_tick();
        assert_eq!(s.media.position, 3.0);
    }

    #[test]
    fn sleep_timer_fades_then_pauses_and_clears() {
        let mut s = session();
        s.dispatch("media.sleep", &json!({"minutes": 1}))
            .expect("arm");
        assert!(s.media.extras.sleep_remaining().is_some());
        s.media.playing = true;
        s.media.extras.sleep = Some(Sleep::At(Instant::now() + Duration::from_secs(5)));
        s.media_audio_tick();
        let fade = s.media.gain.fade();
        assert!(fade > 0.0 && fade < 1.0, "{fade}");
        s.media.extras.sleep = Some(Sleep::At(Instant::now()));
        s.media_audio_tick();
        assert!(!s.media.playing);
        assert_eq!(s.media.extras.sleep, None);
        assert_eq!(s.media.gain.fade(), 1.0);
        s.dispatch("media.sleep", &json!({"minutes": 5}))
            .expect("arm");
        s.dispatch("media.sleep", &json!({"cancel": true}))
            .expect("cancel");
        assert_eq!(s.media.extras.sleep, None);
    }

    #[test]
    fn a_gapless_hand_over_moves_the_queue_and_drops_the_old_items_state() {
        let mut s = session();
        for name in ["a", "b", "c"] {
            s.media
                .queue
                .enqueue(VfsUri::parse(&format!("/{name}.mp3")).expect("uri"))
                .expect("enqueue");
        }
        s.media.queue.select(0).expect("select");
        s.media.error = Some("old".into());
        let next = s.media.queue.peek_next().cloned().expect("next");
        s.media.extras.armed.push((1, next));
        s.media.extras.armed_uri = s.media.queue.peek_next().cloned();
        s.media.extras.loop_a = Some(1.0);
        s.media_gapless_advance(1);
        assert_eq!(s.media.queue.current_index(), Some(1));
        assert_eq!(s.media.error, None);
        assert_eq!(s.media.extras.seen_token(), 1);
        assert_eq!(s.media.extras.loop_a, None);
        assert!(s.media.extras.armed.is_empty());
    }

    #[test]
    fn bad_sleep_params_are_rejected() {
        let mut s = session();
        for params in [
            json!({}),
            json!({"minutes": 0}),
            json!({"minutes": 721}),
            json!({"minutes": "5"}),
            json!({"minutes": 5, "cancel": true}),
            json!({"end_of_item": false}),
            json!(3),
        ] {
            assert!(s.dispatch("media.sleep", &params).is_err(), "{params}");
            assert_eq!(s.media.extras.sleep, None);
        }
    }
}
