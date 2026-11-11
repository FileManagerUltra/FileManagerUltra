//! Subtitles for the current Media video: embedded text tracks and external files next
//! to the video (or chosen by the user). Discovery, extraction and parsing run on
//! short-lived worker threads; the UI only reads the parsed cues.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};

use fm_media::{Cue, Cues, MAX_DELAY_MS, normalize_language};
use fm_preview::subtitles::{ExternalSubtitle, MAX_EXTERNAL};
use fm_vfs::{FmError, VfsUri};
use serde_json::Value;

use crate::{Session, file_cmds::object, registry::CommandSpec};

/// `media.subtitle.delay {step}` moves the delay in these increments.
pub const DELAY_STEP_MS: i64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubtitleSource {
    /// Index into the video's embedded subtitle tracks.
    Embedded(usize),
    /// Index into [`Subtitles::external`].
    External(usize),
}

/// A worker's result channel; dropping it asks the worker to stop.
#[derive(Debug)]
struct Work<T> {
    receiver: Receiver<T>,
    cancelled: Arc<AtomicBool>,
}

impl<T> Drop for Work<T> {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

fn spawn<T: Send + 'static>(
    work: impl FnOnce(&AtomicBool) -> T + Send + 'static,
) -> Result<Work<T>, FmError> {
    let (tx, receiver) = sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = cancelled.clone();
    std::thread::Builder::new()
        .name("fm-subtitles".into())
        .spawn(move || {
            if !flag.load(Ordering::Acquire) {
                let _ = tx.send(work(&flag));
            }
        })
        .map_err(FmError::Io)?;
    Ok(Work {
        receiver,
        cancelled,
    })
}

/// Worker body that produces the cues of one source.
type LoadJob = Box<dyn FnOnce(&AtomicBool) -> Result<Cues, FmError> + Send>;

/// Per-item subtitle state; the preferred language lives for the whole app session.
#[derive(Debug, Default)]
pub struct Subtitles {
    /// External files found beside the video or loaded explicitly.
    pub external: Vec<ExternalSubtitle>,
    pub selected: Option<SubtitleSource>,
    /// What a toggle turns back on.
    last: Option<SubtitleSource>,
    /// Parsed cues of `selected`, once loaded.
    pub cues: Option<Arc<Cues>>,
    /// Positive values show cues later.
    pub delay_ms: i64,
    /// Language of the user's last explicit choice; `None` after turning subtitles off.
    pub preferred_language: Option<String>,
    load: Option<Work<Result<Cues, String>>>,
    discovery: Option<Work<Vec<ExternalSubtitle>>>,
    /// The item whose folder was scanned (or skipped because it is remote).
    scanned: Option<VfsUri>,
    auto_chosen: bool,
}

impl Subtitles {
    pub fn is_loading(&self) -> bool {
        self.load.is_some()
    }

    /// Cues on screen at `position` seconds, with the delay applied.
    pub fn active(&self, position: f64) -> Vec<&Cue> {
        let Some(cues) = &self.cues else {
            return Vec::new();
        };
        if !position.is_finite() || position < 0.0 {
            return Vec::new();
        }
        cues.active((position * 1000.0) as u64, self.delay_ms)
    }

    /// A new queue item: forget everything except the preferred language.
    pub(crate) fn item_changed(&mut self) {
        let preferred_language = self.preferred_language.take();
        *self = Self {
            preferred_language,
            ..Self::default()
        };
    }
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

impl Session {
    fn subtitle_tracks(&self) -> &[fm_preview::SubtitleTrack] {
        self.media
            .video
            .as_ref()
            .map_or(&[], |video| video.info.tracks.subtitles.as_slice())
    }

    fn subtitle_language(&self, source: SubtitleSource) -> Option<String> {
        match source {
            SubtitleSource::Embedded(i) => self
                .subtitle_tracks()
                .get(i)
                .and_then(|track| track.language.as_deref())
                .map(normalize_language),
            SubtitleSource::External(i) => self
                .media
                .subtitles
                .external
                .get(i)
                .and_then(|sub| sub.language.clone()),
        }
    }

    fn subtitle_usable(&self, source: SubtitleSource) -> bool {
        match source {
            SubtitleSource::Embedded(i) => self.subtitle_tracks().get(i).is_some_and(|t| t.text),
            SubtitleSource::External(i) => i < self.media.subtitles.external.len(),
        }
    }

    /// Every selectable source: external files first, then embedded text tracks.
    fn subtitle_sources(&self) -> Vec<SubtitleSource> {
        (0..self.media.subtitles.external.len())
            .map(SubtitleSource::External)
            .chain((0..self.subtitle_tracks().len()).map(SubtitleSource::Embedded))
            .filter(|source| self.subtitle_usable(*source))
            .collect()
    }

    fn subtitle_for_language(&self, language: &str) -> Option<SubtitleSource> {
        self.subtitle_sources()
            .into_iter()
            .find(|source| self.subtitle_language(*source).as_deref() == Some(language))
    }

    /// Show `source` (or nothing) and start loading its cues on a worker.
    pub(crate) fn media_subtitle_select(
        &mut self,
        source: Option<SubtitleSource>,
        explicit: bool,
    ) -> Result<(), FmError> {
        let Some(source) = source else {
            let subs = &mut self.media.subtitles;
            subs.selected = None;
            subs.cues = None;
            subs.load = None;
            if explicit {
                subs.preferred_language = None;
            }
            return Ok(());
        };
        let job: LoadJob = match source {
            SubtitleSource::Embedded(index) => {
                let track = self
                    .subtitle_tracks()
                    .get(index)
                    .ok_or_else(|| bad("subtitle track out of range"))?;
                if !track.text {
                    return Err(bad(
                        "Picture-based subtitles (such as PGS or DVD) cannot be shown yet.",
                    ));
                }
                let path = self
                    .media
                    .video
                    .as_ref()
                    .map(|video| video.info.path.clone())
                    .ok_or_else(|| bad("no video is loaded"))?;
                Box::new(move |cancelled| {
                    fm_preview::subtitles::extract_embedded(&path, index, cancelled)
                })
            }
            SubtitleSource::External(index) => {
                let sub = self
                    .media
                    .subtitles
                    .external
                    .get(index)
                    .cloned()
                    .ok_or_else(|| bad("subtitle file out of range"))?;
                Box::new(move |cancelled| fm_preview::subtitles::load_external(&sub, cancelled))
            }
        };
        let load = spawn(move |cancelled| {
            job(cancelled).map_err(|error| fm_vfs::redact::redact(&error.to_string()))
        })?;
        if explicit {
            self.media.subtitles.preferred_language = self.subtitle_language(source);
        }
        let subs = &mut self.media.subtitles;
        subs.selected = Some(source);
        subs.last = Some(source);
        subs.cues = None;
        subs.load = Some(load);
        Ok(())
    }

    /// Scan the folder, choose by preferred language and collect loaded cues.
    pub(crate) fn media_subtitle_tick(&mut self) -> bool {
        let mut changed = false;
        let current = self
            .media
            .queue
            .current()
            .filter(|uri| crate::media_cmds::is_video(uri))
            .cloned();
        if let Some(uri) = &current
            && self.media.subtitles.scanned.as_ref() != Some(uri)
        {
            self.media.subtitles.scanned = Some(uri.clone());
            if let Some(path) = uri.local_path() {
                match spawn(move |_| fm_preview::subtitles::discover(&path)) {
                    Ok(work) => self.media.subtitles.discovery = Some(work),
                    Err(error) => self.messages.push(&error.to_string()),
                }
            }
        }
        if let Some(work) = &self.media.subtitles.discovery {
            match work.receiver.try_recv() {
                Ok(found) => {
                    let subs = &mut self.media.subtitles;
                    subs.discovery = None;
                    // Files loaded explicitly while scanning stay; duplicates do not.
                    for sub in found {
                        if subs.external.len() < MAX_EXTERNAL
                            && !subs.external.iter().any(|known| known.path == sub.path)
                        {
                            subs.external.push(sub);
                        }
                    }
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.media.subtitles.discovery = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if !self.media.subtitles.auto_chosen
            && current.is_some()
            && self.media.subtitles.discovery.is_none()
            && self.media.video.is_some()
        {
            self.media.subtitles.auto_chosen = true;
            if self.media.subtitles.selected.is_none()
                && let Some(language) = self.media.subtitles.preferred_language.clone()
                && let Some(source) = self.subtitle_for_language(&language)
            {
                if let Err(error) = self.media_subtitle_select(Some(source), false) {
                    self.messages.push(&error.to_string());
                }
                changed = true;
            }
        }
        if let Some(work) = &self.media.subtitles.load {
            let result = match work.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Loading the subtitles did not complete.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let subs = &mut self.media.subtitles;
                subs.load = None;
                match result {
                    Ok(cues) => subs.cues = Some(Arc::new(cues)),
                    Err(error) => {
                        subs.selected = None;
                        subs.cues = None;
                        self.messages.push(&format!("Subtitles not shown: {error}"));
                    }
                }
                changed = true;
            }
        }
        changed
    }
}

fn source_index(value: &Value, len: usize, what: &str) -> Result<usize, FmError> {
    value
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value < len)
        .ok_or_else(|| bad(&format!("{what} out of range")))
}

fn has_subtitle_choice(s: &Session) -> bool {
    s.media.video.is_some()
        && (!s.media.subtitles.external.is_empty() || !s.subtitle_tracks().is_empty())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.track.subtitle",
            label: "Subtitle Track",
            menu: &["Media", "Subtitles"],
            shortcut: None,
            params: "{index: number|null} shows an embedded subtitle track or turns subtitles off; {external: number} shows an external subtitle file; {toggle: true} switches between off and the last (else preferred-language, else first) source",
            enabled: has_subtitle_choice,
            run: |s, params| {
                let map =
                    object(params)?.ok_or_else(|| bad("index, external or toggle required"))?;
                if map.len() != 1 {
                    return Err(bad("supply exactly one of index, external or toggle"));
                }
                if !has_subtitle_choice(s) {
                    return Err(bad("the current video has no subtitles"));
                }
                let target = if let Some(toggle) = map.get("toggle") {
                    if toggle.as_bool() != Some(true) {
                        return Err(bad("toggle must be true"));
                    }
                    if s.media.subtitles.selected.is_some() {
                        None
                    } else {
                        let preferred = s
                            .media
                            .subtitles
                            .preferred_language
                            .clone()
                            .and_then(|language| s.subtitle_for_language(&language));
                        let target = s
                            .media
                            .subtitles
                            .last
                            .filter(|last| s.subtitle_usable(*last))
                            .or(preferred)
                            .or_else(|| s.subtitle_sources().first().copied());
                        Some(
                            target
                                .ok_or_else(|| bad("no subtitles can be shown for this video"))?,
                        )
                    }
                } else if let Some(value) = map.get("index") {
                    if value.is_null() {
                        None
                    } else {
                        let count = s.subtitle_tracks().len();
                        Some(SubtitleSource::Embedded(source_index(
                            value,
                            count,
                            "subtitle track",
                        )?))
                    }
                } else if let Some(value) = map.get("external") {
                    let count = s.media.subtitles.external.len();
                    Some(SubtitleSource::External(source_index(
                        value,
                        count,
                        "subtitle file",
                    )?))
                } else {
                    return Err(bad("only index, external or toggle is accepted"));
                };
                s.media_subtitle_select(target, true)
            },
        },
        CommandSpec {
            id: "media.subtitle.load",
            label: "Load Subtitle File...",
            menu: &["Media", "Load subtitles"],
            shortcut: None,
            params: "{uri: string}: show a local .srt, .vtt, .ass or .ssa file with the current video",
            enabled: |s| s.media.is_video(),
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("uri required"))?;
                if map.keys().any(|key| key != "uri") {
                    return Err(bad("only uri is accepted"));
                }
                let text = map
                    .get("uri")
                    .and_then(Value::as_str)
                    .ok_or_else(|| bad("uri must be a string"))?;
                let path = validate_path(text)?;
                if !s.media.is_video() {
                    return Err(bad("subtitles need a video in the Media pane"));
                }
                let sub = ExternalSubtitle::chosen(&path)?;
                let subs = &mut s.media.subtitles;
                let index = match subs
                    .external
                    .iter()
                    .position(|known| known.path == sub.path)
                {
                    Some(index) => index,
                    None if subs.external.len() >= MAX_EXTERNAL => {
                        return Err(bad("too many subtitle files for this video"));
                    }
                    None => {
                        subs.external.push(sub);
                        subs.external.len() - 1
                    }
                };
                s.media_subtitle_select(Some(SubtitleSource::External(index)), true)
            },
        },
        CommandSpec {
            id: "media.subtitle.delay",
            label: "Subtitle Delay",
            menu: &["Media", "Subtitle delay"],
            shortcut: None,
            params: "{ms: integer} sets the delay (positive shows subtitles later, ±600000); {step: integer} moves it by that many 100 ms steps",
            enabled: |s| s.media.subtitles.selected.is_some(),
            run: |s, params| {
                let map = object(params)?.ok_or_else(|| bad("ms or step required"))?;
                if map.len() != 1 {
                    return Err(bad("supply exactly one of ms or step"));
                }
                let integer = |key: &str| {
                    map.get(key)
                        .and_then(Value::as_i64)
                        .filter(|value| value.unsigned_abs() <= MAX_DELAY_MS.unsigned_abs())
                        .ok_or_else(|| bad("delay must be an integer within ±600000 ms"))
                };
                let delay = if map.contains_key("ms") {
                    integer("ms")?
                } else if map.contains_key("step") {
                    let step = integer("step")?;
                    s.media
                        .subtitles
                        .delay_ms
                        .checked_add(step.saturating_mul(DELAY_STEP_MS))
                        .filter(|value| value.unsigned_abs() <= MAX_DELAY_MS.unsigned_abs())
                        .ok_or_else(|| bad("subtitle delay is limited to ±10 minutes"))?
                } else {
                    return Err(bad("only ms or step is accepted"));
                };
                s.media.subtitles.delay_ms = delay;
                Ok(())
            },
        },
    ]
}

/// A local absolute path with a subtitle extension (no I/O).
pub fn validate_path(text: &str) -> Result<std::path::PathBuf, FmError> {
    if text.is_empty() || text.len() > 4096 {
        return Err(bad("choose a subtitle file"));
    }
    let path = VfsUri::parse(text)?
        .local_path()
        .ok_or_else(|| bad("subtitles must be a local file"))?;
    ExternalSubtitle::chosen(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media_track_cmds::tests::session_with;
    use fm_preview::SubtitleTrack;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn tracks() -> Vec<SubtitleTrack> {
        vec![
            SubtitleTrack {
                label: "English".into(),
                text: true,
                language: Some("eng".into()),
            },
            SubtitleTrack {
                label: "PGS".into(),
                text: false,
                language: Some("nld".into()),
            },
            SubtitleTrack {
                label: "Deutsch".into(),
                text: true,
                language: Some("ger".into()),
            },
        ]
    }

    fn wait(s: &mut Session, done: impl Fn(&Session) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(s) {
            assert!(Instant::now() < deadline, "timed out");
            s.media_subtitle_tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn srt(dir: &std::path::Path, name: &str, text: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, format!("00:00:01,000 --> 00:00:03,000\n{text}\n")).expect("write");
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn external_file_next_to_the_video_is_listed_and_shown() {
        let dir = tempfile::tempdir().expect("tmp");
        let video = dir.path().join("movie.mkv");
        std::fs::write(&video, b"").expect("video");
        srt(dir.path(), "movie.en.srt", "Hello");
        let mut s = session_with(&video.to_string_lossy(), tracks());
        wait(&mut s, |s| {
            s.media.subtitles.discovery.is_none() && s.media.subtitles.auto_chosen
        });
        assert_eq!(s.media.subtitles.external.len(), 1);
        assert_eq!(
            s.media.subtitles.selected, None,
            "off without a preferred language"
        );
        s.dispatch("media.track.subtitle", &json!({"external": 0}))
            .expect("select");
        wait(&mut s, |s| s.media.subtitles.cues.is_some());
        let texts: Vec<_> = s
            .media
            .subtitles
            .active(1.5)
            .iter()
            .map(|c| c.text.clone())
            .collect();
        assert_eq!(texts, ["Hello"]);
        assert_eq!(s.media.subtitles.preferred_language.as_deref(), Some("en"));
    }

    #[test]
    fn preferred_language_carries_to_the_next_item_and_off_clears_it() {
        let dir = tempfile::tempdir().expect("tmp");
        let mut s = session_with("/video/a.mkv", tracks());
        s.media.subtitles.preferred_language = Some("de".into());
        s.media.subtitles.scanned = s.media.queue.current().cloned();
        s.media_subtitle_tick();
        assert_eq!(
            s.media.subtitles.selected,
            Some(SubtitleSource::Embedded(2))
        );
        // A preference nothing matches leaves subtitles off.
        s.media.subtitles.item_changed();
        s.media.subtitles.preferred_language = Some("fr".into());
        s.media.subtitles.scanned = s.media.queue.current().cloned();
        s.media_subtitle_tick();
        assert_eq!(s.media.subtitles.selected, None);
        assert_eq!(s.media.subtitles.preferred_language.as_deref(), Some("fr"));
        let file = srt(dir.path(), "x.srt", "x");
        s.dispatch("media.subtitle.load", &json!({"uri": file}))
            .expect("load");
        s.dispatch("media.track.subtitle", &json!({"index": null}))
            .expect("off");
        assert_eq!(s.media.subtitles.preferred_language, None);
    }

    #[test]
    fn toggle_remembers_the_last_source_and_bitmap_tracks_are_refused() {
        let mut s = session_with("/video/a.mkv", tracks());
        let err = s
            .dispatch("media.track.subtitle", &json!({"index": 1}))
            .expect_err("pgs");
        assert!(err.to_string().contains("Picture-based"));
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("on");
        assert_eq!(
            s.media.subtitles.selected,
            Some(SubtitleSource::Embedded(0))
        );
        s.dispatch("media.track.subtitle", &json!({"index": 2}))
            .expect("choose");
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("off");
        assert_eq!(s.media.subtitles.selected, None);
        s.dispatch("media.track.subtitle", &json!({"toggle": true}))
            .expect("on again");
        assert_eq!(
            s.media.subtitles.selected,
            Some(SubtitleSource::Embedded(2))
        );
    }

    #[test]
    fn a_malformed_file_is_reported_and_playback_continues_without_it() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("broken.srt");
        std::fs::write(&path, "no cues here").expect("write");
        let mut s = session_with("/video/a.mkv", tracks());
        s.dispatch(
            "media.subtitle.load",
            &json!({"uri": path.to_string_lossy()}),
        )
        .expect("queued");
        wait(&mut s, |s| !s.media.subtitles.is_loading());
        assert_eq!(s.media.subtitles.selected, None);
        assert!(s.media.subtitles.cues.is_none());
        assert!(
            s.messages
                .recent()
                .any(|m| m.text.contains("no subtitle cues"))
        );
        assert!(s.media.subtitles.active(2.0).is_empty());
    }

    #[test]
    fn delay_moves_in_steps_and_rejects_bad_values() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = srt(dir.path(), "d.srt", "Line");
        let mut s = session_with("/video/a.mkv", tracks());
        assert!(!s.command_enabled("media.subtitle.delay"));
        s.dispatch("media.subtitle.load", &json!({"uri": file}))
            .expect("load");
        wait(&mut s, |s| s.media.subtitles.cues.is_some());
        s.dispatch("media.subtitle.delay", &json!({"step": 5}))
            .expect("step");
        assert_eq!(s.media.subtitles.delay_ms, 500);
        // The cue at 1.0 s now appears at 1.5 s.
        assert!(s.media.subtitles.active(1.4).is_empty());
        assert_eq!(s.media.subtitles.active(1.5).len(), 1);
        s.dispatch("media.subtitle.delay", &json!({"step": -7}))
            .expect("back");
        assert_eq!(s.media.subtitles.delay_ms, -200);
        s.dispatch("media.subtitle.delay", &json!({"ms": 600_000}))
            .expect("max");
        for params in [
            json!({"step": 1}),
            json!({"ms": 600_001}),
            json!({"ms": 1.5}),
            json!({"ms": "100"}),
            json!({"step": i64::MIN}),
            json!({"ms": 0, "step": 1}),
            json!({"x": 1}),
            json!({}),
            json!(null),
        ] {
            assert!(
                s.dispatch("media.subtitle.delay", &params).is_err(),
                "{params}"
            );
            assert_eq!(s.media.subtitles.delay_ms, 600_000);
        }
    }

    #[test]
    fn bad_selection_and_load_params_are_errors() {
        let mut s = session_with("/video/a.mkv", tracks());
        for (command, params) in [
            ("media.track.subtitle", json!({"index": 3})),
            ("media.track.subtitle", json!({"index": -1})),
            ("media.track.subtitle", json!({"index": "0"})),
            ("media.track.subtitle", json!({"external": 0})),
            ("media.track.subtitle", json!({"toggle": false})),
            ("media.track.subtitle", json!({"toggle": true, "index": 0})),
            ("media.track.subtitle", json!({})),
            ("media.track.subtitle", json!(null)),
            ("media.subtitle.load", json!({"uri": "relative.srt"})),
            ("media.subtitle.load", json!({"uri": "/x/movie.txt"})),
            (
                "media.subtitle.load",
                json!({"uri": "sftp://user:secret@host/a.srt"}),
            ),
            ("media.subtitle.load", json!({"uri": 5})),
            ("media.subtitle.load", json!({"uri": "/a.srt", "x": 1})),
            ("media.subtitle.load", json!({"uri": "/".repeat(5000)})),
            ("media.subtitle.load", json!(null)),
        ] {
            let result = s.dispatch(command, &params);
            assert!(result.is_err(), "{command} {params}");
            if let Err(error) = result {
                assert!(!error.to_string().contains("secret"));
            }
            assert_eq!(s.media.subtitles.selected, None);
        }
        s.media.video = None;
        s.media.queue.clear();
        assert!(!s.command_enabled("media.track.subtitle"));
        assert!(!s.command_enabled("media.subtitle.load"));
        assert!(
            s.dispatch("media.subtitle.load", &json!({"uri": "/a.srt"}))
                .is_err()
        );
    }
}
