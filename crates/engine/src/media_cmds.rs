//! Session-owned local playback. File, probe and device I/O stays on workers.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{SystemTime, UNIX_EPOCH};

use fm_layout::{Axis, PaneKind};
use fm_media::{AudioInfo, AudioPlayer, Gain, PlaybackState, PlaybackStatus, Queue, RepeatMode};
use fm_vfs::{FmError, VfsUri};

use crate::Session;
use crate::audio_owner::AudioOwner;
use crate::file_cmds::{Obj, object};
use crate::media_entry_cmds::{has_sources, sources};
use crate::registry::{CommandSpec, Params};

#[derive(Debug)]
struct VideoProbe {
    receiver: Receiver<Result<fm_preview::VideoInfo, String>>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for VideoProbe {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// A decoded cover and its PNG export for the OS media controls, or why decoding failed.
pub(crate) type CoverResult = Result<(fm_preview::ImageDoc, Option<std::path::PathBuf>), String>;

/// Queue rows just added by one request, for the playlist to select and scroll to once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaReveal {
    pub start: usize,
    pub len: usize,
    /// Bumped for every addition; a pane remembers the last one it showed.
    pub seq: u64,
}

#[derive(Debug)]
pub struct MediaSession {
    pub queue: Queue,
    pub playing: bool,
    pub position: f64,
    pub volume: f32,
    pub muted: bool,
    /// Speed of the current item (0.5–3×, pitch kept).
    pub speed: f64,
    /// Speed for items that are not audiobooks; books remember their own.
    pub session_speed: f64,
    pub info: Option<Arc<AudioInfo>>,
    pub cover: Option<Arc<fm_preview::ImageDoc>>,
    pub error: Option<String>,
    pub video: Option<crate::video_cmds::VideoState>,
    /// Subtitle sources, selection, cues and delay of the current video.
    pub subtitles: crate::media_subtitle_cmds::Subtitles,
    /// Downloads of remote items, played from their local copy.
    pub spool: crate::media_spool::Spool,
    pub extras: crate::media_audio_cmds::AudioExtras,
    pub(crate) player: Option<AudioPlayer>,
    video_probe: Option<VideoProbe>,
    pub(crate) video_width: u32,
    pub(crate) gain: Gain,
    pub(crate) failed_items: usize,
    /// Last position per folder (audiobook), newest last.
    pub resume: Vec<crate::media_resume::ResumePoint>,
    /// Audiobooks and videos: chapters, timeline, remembered positions, bookmarks.
    pub books: crate::media_book::BookExtras,
    pub(crate) cover_job: Option<(Receiver<CoverResult>, crate::jobs::JobId)>,
    /// The decoded cover as an owner-only PNG, for the OS media controls.
    pub cover_file: Option<std::path::PathBuf>,
    /// The rows the last add request put in the queue (see [`MediaReveal`]).
    pub reveal: Option<MediaReveal>,
}

impl Default for MediaSession {
    fn default() -> Self {
        Self {
            queue: Queue::default(),
            playing: false,
            position: 0.0,
            volume: 100.0,
            muted: false,
            speed: 1.0,
            session_speed: 1.0,
            info: None,
            cover: None,
            error: None,
            video: None,
            subtitles: Default::default(),
            spool: Default::default(),
            extras: Default::default(),
            player: None,
            video_probe: None,
            video_width: 640,
            gain: Gain::default(),
            failed_items: 0,
            resume: Vec::new(),
            books: Default::default(),
            cover_job: None,
            cover_file: None,
            reveal: None,
        }
    }
}

impl MediaSession {
    /// Ask the playlist to select and show `len` rows from `start` (nothing when `len` is 0).
    pub(crate) fn note_added(&mut self, start: usize, len: usize) {
        if len == 0 {
            return;
        }
        let seq = self.reveal.map_or(1, |r| r.seq.wrapping_add(1));
        self.reveal = Some(MediaReveal { start, len, seq });
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.playing
            || self.cover_job.is_some()
            || (self.player.is_some() && self.info.is_none())
            || self.video_probe.is_some()
            || self.spool.is_pending()
            || self
                .video
                .as_ref()
                .is_some_and(|video| !video.player_is_idle())
    }

    pub fn gain_eq(&self) -> fm_media::EqSettings {
        self.gain.eq()
    }

    pub fn duration(&self) -> Option<f64> {
        self.video
            .as_ref()
            .map(|video| video.info.duration)
            .filter(|duration| *duration > 0.0)
            .or_else(|| self.info.as_ref().and_then(|info| info.duration))
            .or_else(|| self.cached_length())
    }

    /// An audiobook's remembered speed, else the session speed.
    pub fn speed_for_current(&self) -> f64 {
        self.queue
            .current_index()
            .and_then(|index| self.book_ref(index))
            .filter(|book| !book.video)
            .and_then(|book| self.state_of(&book.key).and_then(|state| state.speed))
            .unwrap_or(self.session_speed)
    }

    pub fn is_video(&self) -> bool {
        self.queue.current().is_some_and(is_video)
    }

    pub(crate) fn pause(&mut self) {
        self.playing = false;
        if let Some(player) = &self.player {
            player.set_paused(true);
        }
        if let Some(video) = &mut self.video {
            video.pause();
        }
    }

    fn stop(&mut self) {
        self.pause();
        self.player = None;
        self.video_probe = None;
        self.spool.cancel_except(None);
        self.position = 0.0;
        if let Some(video) = &mut self.video {
            video.pos = 0.0;
        }
        self.failed_items = 0;
        self.extras.item_changed();
    }

    fn start(&mut self) -> Result<(), FmError> {
        let uri = self
            .queue
            .current()
            .ok_or_else(|| bad("the media queue has no current item"))?;
        validate_media_source(uri)?;
        let path = self
            .playable(uri)
            .and_then(|uri| uri.local_path())
            .ok_or_else(|| bad("media needs a local file"))?;
        self.speed = self.speed_for_current();
        let player = AudioPlayer::start_with(path, self.gain.clone(), self.position, self.speed)?;
        self.extras.item_changed();
        self.player = Some(player);
        self.playing = true;
        self.error = None;
        Ok(())
    }

    /// The local file to play for `uri`: itself, or its finished spool copy.
    pub(crate) fn playable(&self, uri: &VfsUri) -> Option<VfsUri> {
        if uri.local_path().is_some() {
            return Some(uri.clone());
        }
        let path = self.spool.path(uri)?;
        VfsUri::parse(&path.to_string_lossy()).ok()
    }

    /// Download progress of the current remote item, for the pane.
    pub fn spool_progress(&self) -> Option<(u64, u64)> {
        self.spool.progress()
    }

    pub(crate) fn reset_item(&mut self) {
        let current = self.queue.current().cloned();
        self.spool.cancel_except(current.as_ref());
        self.player = None;
        self.position = 0.0;
        self.info = None;
        self.video = None;
        self.subtitles.item_changed();
        self.video_probe = None;
        self.cover = None;
        self.cover_file = None;
        self.cover_job = None;
        self.error = None;
        self.extras.item_changed();
    }

    /// Manual skips and failed items must escape repeat-one.
    fn skip(&mut self) -> bool {
        let repeat = self.queue.repeat();
        if repeat == RepeatMode::One {
            self.queue.set_repeat(RepeatMode::Off);
        }
        let found = self.queue.next_item().is_some();
        self.queue.set_repeat(repeat);
        found
    }
}

impl Session {
    /// Play newly added items from `first`, continuing a remembered book or video.
    pub(crate) fn media_play_appended(&mut self, first: usize) -> Result<(), FmError> {
        self.media_play_row(first, false)
    }

    /// Select queue row `index` and play it; a remembered book or video continues where it
    /// was left unless `from_start`.
    pub(crate) fn media_play_row(&mut self, index: usize, from_start: bool) -> Result<(), FmError> {
        let position = if from_start {
            0.0
        } else {
            self.media.remembered_position(index).unwrap_or(0.0)
        };
        self.media_start_at(index, position)
    }

    pub(crate) fn media_start_at(&mut self, first: usize, position: f64) -> Result<(), FmError> {
        self.show_media_pane()?;
        self.media.queue.select(first)?;
        self.media.reset_item();
        self.media.position = position;
        self.media.failed_items = 0;
        self.media_resume()
    }

    pub(crate) fn media_set_restored(&mut self, queue: Queue, volume: f32) -> Result<(), FmError> {
        self.media.gain.set_volume(volume)?;
        self.media.queue = queue;
        self.media.volume = volume;
        Ok(())
    }

    pub fn media_pane(&self) -> Option<fm_layout::PaneId> {
        self.layout.panes().into_iter().find(|pane| {
            self.layout
                .state(*pane)
                .is_some_and(|st| st.kind == PaneKind::Media)
        })
    }

    pub(crate) fn show_media_pane(&mut self) -> Result<(), FmError> {
        if let Some(pane) = self.media_pane() {
            return self.layout.focus_pane(pane);
        }
        let pane = self
            .layout
            .split(self.layout.focused(), Axis::Horizontal, 0.5)?;
        if let Some(state) = self.layout.state_mut(pane) {
            state.kind = PaneKind::Media;
        }
        Ok(())
    }

    pub(crate) fn media_stop(&mut self) {
        self.media.stop();
        self.release_audio(AudioOwner::Media);
        self.media_fullscreen_leave(true);
    }

    pub(crate) fn media_resume(&mut self) -> Result<(), FmError> {
        if self.media.queue.current().is_none() && self.media.queue.next_item().is_none() {
            return Err(bad("the media queue is empty"));
        }
        if let Some(uri) = self.media.queue.current() {
            validate_media_source(uri)?;
        }
        if self.media_pane().is_none() && !self.prefs.media_background_audio {
            self.show_media_pane()?;
        }
        self.claim_audio(AudioOwner::Media);
        let waiting = match self.media.queue.current().cloned() {
            Some(uri) if self.media.playable(&uri).is_none() => {
                if self.media.spool.is_pending_for(&uri) {
                    Ok(true)
                } else {
                    self.media_spool_begin(uri).map(|()| true)
                }
            }
            _ => Ok(false),
        };
        let result = if let Err(error) = waiting {
            Err(error)
        } else if waiting.is_ok_and(|waiting| waiting) {
            self.media.playing = true;
            self.media.error = None;
            Ok(())
        } else if self.media.is_video() {
            self.media.playing = true;
            self.media.error = None;
            if let Some(video) = &mut self.media.video {
                video.playing = true;
                video.error = None;
                if video.info.duration > 0.0 && video.pos >= video.info.duration {
                    video.pos = 0.0;
                    self.media.position = 0.0;
                }
                video.restart();
            } else if self.media.video_probe.is_none() {
                self.media_video_probe()?;
            }
            Ok(())
        } else if let Some(player) = &self.media.player {
            player.set_paused(false);
            self.media.playing = true;
            Ok(())
        } else {
            self.media.start()
        };
        if result.is_err() {
            self.media.playing = false;
            self.release_audio(AudioOwner::Media);
        }
        result
    }

    /// Probe the current video on the local lane; the result becomes `media.video`.
    fn media_video_probe(&mut self) -> Result<(), FmError> {
        let uri = self
            .media
            .queue
            .current()
            .and_then(|uri| self.media.playable(uri))
            .ok_or_else(|| bad("the media queue has no current item"))?;
        let (tx, rx) = sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let work_cancelled = cancelled.clone();
        self.jobs.submit(crate::jobs::JobLane::Local, move |ctx| {
            if ctx.is_cancelled() || work_cancelled.load(Ordering::Acquire) {
                return Ok(());
            }
            let result = fm_preview::video::playback_info(&uri, &work_cancelled)
                .map_err(|error| fm_vfs::redact::redact(&error.to_string()));
            let _ = tx.send(result);
            Ok(())
        });
        self.media.video_probe = Some(VideoProbe {
            receiver: rx,
            cancelled,
        });
        Ok(())
    }

    /// Called after pane commands and on each poll, including workspace switches.
    pub(crate) fn media_visibility_changed(&mut self) {
        self.media_fullscreen_check();
        if self.media_pane().is_none() && !self.prefs.media_background_audio {
            self.media_stop();
        }
    }

    pub(crate) fn media_tick(&mut self) -> bool {
        self.media_visibility_changed();
        let mut changed = self.media_spool_poll();
        changed |= self.media_audio_tick();
        changed |= self.media_book_tick();
        changed |= self.media_subtitle_tick();
        // A selected but idle video shows its first frame instead of a black view.
        if self.media.is_video()
            && !self.media.playing
            && self.media.video.is_none()
            && self.media.video_probe.is_none()
            && self.media.error.is_none()
            && self.media_pane().is_some()
            && self
                .media
                .queue
                .current()
                .is_some_and(|uri| self.media.playable(uri).is_some())
        {
            let _ = self.media_video_probe();
        }
        if let Some(probe) = &self.media.video_probe {
            let result = match probe.receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("Video probing did not complete.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.media.video_probe = None;
                match result {
                    Ok(info) => {
                        let mut video = crate::video_cmds::VideoState::with_gain(
                            info,
                            self.media.playing,
                            self.media.gain.clone(),
                        );
                        video.width = Some(self.media.video_width);
                        video.pos = self.media.position;
                        self.media.speed = self.media.speed_for_current();
                        video.speed = self.media.speed;
                        if !self.media.playing {
                            // Decode one still frame without playing.
                            video.restart();
                        }
                        self.media.video = Some(video);
                    }
                    Err(error) => {
                        return self.media_apply_status(PlaybackStatus {
                            token: 0,
                            state: PlaybackState::Failed,
                            info: None,
                            position: self.media.position,
                            error: Some(error),
                        });
                    }
                }
                changed = true;
            }
        }
        let overlay = self.media_subtitle_overlay();
        let hardware = self.prefs.media_hardware_decoding;
        if let Some(video) = &mut self.media.video {
            video.set_hardware(hardware);
            video.set_overlay(overlay);
            changed |= crate::video_cmds::pump(video);
            if let Some(reason) = video.take_fallback() {
                self.messages.push(&reason);
            }
            let state = if video.error.is_some() {
                PlaybackState::Failed
            } else if self.media.playing && !video.playing {
                PlaybackState::Finished
            } else if self.media.playing {
                PlaybackState::Playing
            } else {
                PlaybackState::Paused
            };
            let status = PlaybackStatus {
                token: 0,
                state,
                info: None,
                position: video.pos,
                error: video.error.clone(),
            };
            return self.media_apply_status(status) || changed;
        }
        if let Some((rx, _)) = &self.media.cover_job {
            match rx.try_recv() {
                Ok(result) => {
                    self.media.cover_job = None;
                    match result {
                        Ok((cover, file)) => {
                            self.media.cover = Some(Arc::new(cover));
                            self.media.cover_file = file;
                        }
                        Err(error) => self.messages.push(&error),
                    }
                    changed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.media.cover_job = None;
                    self.messages
                        .push("Media cover-art decoding did not complete.");
                    changed = true;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        let Some(player) = &self.media.player else {
            return changed;
        };
        let status = player.status().unwrap_or_else(|err| PlaybackStatus {
            token: 0,
            state: PlaybackState::Failed,
            info: None,
            position: self.media.position,
            error: Some(err.to_string()),
        });
        self.media_apply_status(status) || changed
    }

    pub(crate) fn media_apply_status(&mut self, status: PlaybackStatus) -> bool {
        if status.token != 0 && status.token != self.media.extras.seen_token() {
            self.media_gapless_advance(status.token);
        }
        let changed = (self.media.position - status.position).abs() > 0.02
            || (self.media.info.is_none() && status.info.is_some());
        self.media.position = status.position;
        if status.state == PlaybackState::Playing {
            self.media.note_book();
        }
        if self.media.info.is_none()
            && let Some(info) = &status.info
        {
            if let Some((_, job)) = self.media.cover_job.take() {
                self.jobs.cancel(job);
            }
            if let Some(cover) = &info.tags.cover
                && let Some(uri) = self.media.queue.current()
            {
                let bytes = cover.bytes.clone();
                let entry = fm_vfs::Entry::new(uri.clone(), fm_vfs::EntryKind::File);
                let (tx, rx) = sync_channel(1);
                let job = self.jobs.submit(crate::jobs::JobLane::Local, move |ctx| {
                    if ctx.is_cancelled() {
                        return Ok(());
                    }
                    let limits = fm_preview::Limits {
                        image_edge: 512,
                        image_pixels: 4_000_000,
                        ..Default::default()
                    };
                    let result = match fm_preview::image_prev::decode_still(&entry, &bytes, &limits)
                    {
                        fm_preview::Preview::Image(image) => {
                            let file = crate::media_cover_file::export(&image);
                            Ok((image, file))
                        }
                        _ => Err("Embedded media cover art could not be decoded safely.".into()),
                    };
                    // Replacing the current item drops the receiver; its artwork is no longer needed.
                    let _ = tx.send(result);
                    Ok(())
                });
                self.media.cover_job = Some((rx, job));
            }
        }
        if status.info.is_some() {
            self.media.info = status.info;
        }
        if !matches!(
            status.state,
            PlaybackState::Finished | PlaybackState::Failed
        ) {
            if status.state == PlaybackState::Playing && status.position > 0.1 {
                self.media.failed_items = 0;
            }
            return changed;
        }
        // A paused owner's final status must not restart playback after Preview took over.
        let was_playing = self.media.playing;
        self.media.player = None;
        if let Some(video) = &mut self.media.video {
            video.pause();
            video.error = None;
        }
        self.media.playing = false;
        self.release_audio(AudioOwner::Media);
        let failed = status.state == PlaybackState::Failed;
        if !failed {
            self.media.forget_finished_book();
        }
        if !failed
            && was_playing
            && self.media.extras.sleep == Some(crate::media_audio_cmds::Sleep::EndOfItem)
        {
            // Sleep timer set to the end of this item: line up the next one, but stay silent.
            self.media_sleep_clear();
            if self.media.queue.next_item().is_some() {
                self.media.reset_item();
            }
            self.media_fullscreen_leave(true);
            return true;
        }
        if failed {
            let name = self
                .media
                .queue
                .current()
                .and_then(VfsUri::file_name)
                .unwrap_or("Media item");
            let error = format!(
                "{name}: {}",
                status.error.as_deref().unwrap_or("media playback failed")
            );
            if self.media.is_video()
                && (fm_preview::video::find_tool("ffmpeg").is_none()
                    || fm_preview::video::find_tool("ffprobe").is_none())
            {
                self.ffmpeg.prompt = true;
            }
            self.messages.push(&error);
            self.media.error = Some(error);
            self.media.failed_items += 1;
        } else {
            self.media.failed_items = 0;
        }
        if !failed
            && was_playing
            && self.media.queue.ends_group()
            && let Some(start) = self.media.queue.current_group_start()
        {
            // A folder is a playlist in the playlist: stop at its end, ready at its first item.
            if self.media.queue.select(start).is_ok() {
                self.media.reset_item();
            }
            self.media_fullscreen_leave(true);
            return true;
        }
        if !was_playing || self.media.failed_items >= self.media.queue.items().len() {
            if was_playing {
                self.media_fullscreen_leave(true);
            }
            return true;
        }
        let next = if failed {
            self.media.skip()
        } else {
            self.media.queue.next_item().is_some()
        };
        if !next {
            self.media_fullscreen_leave(true);
        }
        if next {
            self.media.reset_item();
            if let Err(err) = self.media_resume() {
                self.media.error = Some(err.to_string());
                self.messages.push(&err.to_string());
            }
        }
        true
    }
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

pub(crate) fn is_video(uri: &VfsUri) -> bool {
    uri.file_name().is_some_and(|name| {
        fm_preview::thumbs::classify(name) == fm_preview::thumbs::ThumbKind::Video
            || fm_preview::video::is_disc_image(name)
    })
}

/// Folder play, playlists and the saved queue accept local files only.
pub(crate) fn validate_media(uri: &VfsUri) -> Result<(), FmError> {
    if uri.local_path().is_none() {
        return Err(bad("Choose a local media file."));
    }
    validate_media_source(uri)
}

/// Any backend's file with a supported extension; remote files are downloaded before playing.
pub(crate) fn validate_media_source(uri: &VfsUri) -> Result<(), FmError> {
    let extension = uri
        .file_name()
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, extension)| extension.to_ascii_lowercase());
    if !is_video(uri)
        && !matches!(
            extension.as_deref(),
            Some(
                "mp3" | "flac" | "ogg" | "oga" | "wav" | "aac" | "m4a" | "alac" | "m4b"
                    // Decoded by ffmpeg when Symphonia cannot read them.
                    | "wma" | "opus" | "ape" | "wv" | "aif" | "aiff" | "ac3" | "dts" | "mka"
                    | "amr" | "mpc" | "tta"
            )
        )
    {
        return Err(bad("Choose a supported audio or video file."));
    }
    // A multi-gigabyte image is read in place, never downloaded to the spool.
    if uri
        .file_name()
        .is_some_and(fm_preview::video::is_disc_image)
        && uri.local_path().is_none()
    {
        return Err(bad("DVD images (.iso) must be on this computer to play."));
    }
    Ok(())
}

fn map(params: &Params) -> Result<&Obj, FmError> {
    object(params)?.ok_or_else(|| bad("named media parameters required"))
}

fn allowed(map: Option<&Obj>, keys: &[&str]) -> Result<(), FmError> {
    if map.is_some_and(|map| map.keys().any(|key| !keys.contains(&key.as_str()))) {
        return Err(bad("unknown media parameter"));
    }
    Ok(())
}

fn no_params(params: &Params) -> Result<(), FmError> {
    if object(params)?.is_some_and(|m| !m.is_empty()) {
        return Err(bad("this command takes no parameters"));
    }
    Ok(())
}

fn index(map: &Obj, key: &str) -> Result<usize, FmError> {
    map.get(key)
        .and_then(Params::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| bad("queue index must be a non-negative integer"))
}

fn on(params: &Params, current: bool) -> Result<bool, FmError> {
    let map = object(params)?;
    if map.is_some_and(|map| map.keys().any(|key| key != "on")) {
        return Err(bad("only on: bool is accepted"));
    }
    map.and_then(|m| m.get("on"))
        .map(|v| v.as_bool().ok_or_else(|| bad("on must be true or false")))
        .unwrap_or(Ok(!current))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.play",
            label: "Play in Media",
            menu: &["Media", "Play"],
            shortcut: None,
            params: "{index?: number, from_start?: bool, uris?: string[], pane?: number}: play a queue row in place (a remembered audiobook or video continues unless from_start), or insert source media after current and play (Preview transfers position)",
            enabled: |s| !s.media.queue.items().is_empty() || has_sources(s),
            run: |s, params| {
                let map = object(params)?;
                allowed(map, &["index", "uris", "pane", "from_start"])?;
                let from_start = match map.and_then(|m| m.get("from_start")) {
                    None => false,
                    Some(value) => value
                        .as_bool()
                        .ok_or_else(|| bad("from_start must be true or false"))?,
                };
                let selected = map
                    .filter(|m| m.contains_key("index"))
                    .map(|m| index(m, "index"))
                    .transpose()?;
                if selected.is_some()
                    && map.is_some_and(|m| m.contains_key("uris") || m.contains_key("pane"))
                {
                    return Err(bad("choose either index or source media"));
                }

                let items = if map.is_some_and(|m| m.contains_key("uris") || m.contains_key("pane"))
                    || (selected.is_none()
                        && s.pane_kind(s.layout.focused()) == Some(PaneKind::Preview))
                    || (selected.is_none() && !crate::file_cmds::focused_selection(s).is_empty())
                {
                    Some(sources(s, map)?)
                } else {
                    None
                };
                if let Some(index) = selected {
                    let uri = s
                        .media
                        .queue
                        .items()
                        .get(index)
                        .ok_or_else(|| bad("media queue index out of range"))?;
                    validate_media_source(uri)?;
                }
                if selected.is_none() && items.is_none() && s.media.queue.items().is_empty() {
                    return Err(bad("the media queue is empty"));
                }
                s.show_media_pane()?;
                if let Some(input) = items {
                    for uri in &input.items {
                        s.pause_previews_of(uri);
                    }
                    let first = s.media_insert_next(input.items)?;
                    s.media_start_at(first, input.position)?;
                    s.stop_handed_off_preview(input.preview);
                    return Ok(());
                } else if let Some(index) = selected {
                    return s.media_play_row(index, from_start);
                }
                s.media.failed_items = 0;
                s.media_resume()
            },
        },
        CommandSpec {
            id: "media.enqueue",
            label: "Add to Queue",
            menu: &["Media", "Add to queue"],
            shortcut: None,
            params: "{uris?: string[], pane?: number}: append local media from Files or Preview without changing playback",
            enabled: has_sources,
            run: |s, params| {
                let map = object(params)?;
                allowed(map, &["uris", "pane"])?;
                let items = sources(s, map)?;
                s.show_media_pane()?;
                let first = s.media.queue.items().len();
                let count = items.items.len();
                for uri in items.items {
                    s.media.queue.enqueue(uri)?;
                }
                s.media.note_added(first, count);
                Ok(())
            },
        },
        CommandSpec {
            id: "media.pause",
            label: "Pause",
            menu: &["Media", "Pause"],
            shortcut: None,
            params: "none",
            enabled: |s| s.media.playing,
            run: |s, p| {
                no_params(p)?;
                s.media.pause();
                s.release_audio(AudioOwner::Media);
                Ok(())
            },
        },
        CommandSpec {
            id: "media.toggle",
            label: "Play / Pause",
            menu: &["Media", "Play / pause"],
            shortcut: None,
            params: "none",
            enabled: |s| !s.media.queue.items().is_empty(),
            run: |s, p| {
                no_params(p)?;
                if s.media.playing {
                    s.media.pause();
                    s.release_audio(AudioOwner::Media);
                    Ok(())
                } else {
                    s.media.failed_items = 0;
                    s.media_resume()
                }
            },
        },
        CommandSpec {
            id: "media.stop",
            label: "Stop",
            menu: &["Media", "Stop"],
            shortcut: None,
            params: "none",
            enabled: |s| s.media.queue.current().is_some(),
            run: |s, p| {
                no_params(p)?;
                s.media_stop();
                Ok(())
            },
        },
        CommandSpec {
            id: "media.next",
            label: "Next",
            menu: &["Media", "Next"],
            shortcut: None,
            params: "none",
            enabled: |s| !s.media.queue.items().is_empty(),
            run: |s, p| {
                no_params(p)?;
                if s.media.skip() {
                    s.media.reset_item();
                    s.media.failed_items = 0;
                    s.media_resume()
                } else {
                    s.media_stop();
                    Ok(())
                }
            },
        },
        CommandSpec {
            id: "media.prev",
            label: "Previous",
            menu: &["Media", "Previous"],
            shortcut: None,
            params: "none",
            enabled: |s| s.media.queue.current().is_some(),
            run: |s, p| {
                no_params(p)?;
                if s.media.queue.previous_item().is_none() {
                    return Err(bad("no previous media item"));
                }
                s.media.reset_item();
                s.media.failed_items = 0;
                s.media_resume()
            },
        },
        CommandSpec {
            id: "media.seek",
            label: "Seek",
            menu: &["Media", "Seek"],
            shortcut: None,
            params: "{to: number} or {by: number}: seconds, clamped to the current item",
            enabled: |s| s.media.queue.current().is_some(),
            run: |s, p| {
                let map = map(p)?;
                let absolute = map.get("to");
                let relative = map.get("by");
                let (value, relative) = match (absolute, relative) {
                    (Some(v), None) => (v, false),
                    (None, Some(v)) => (v, true),
                    _ => return Err(bad("supply exactly one of to or by")),
                };
                let value = value
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| bad("seek requires finite seconds"))?;
                if !relative && value < 0.0 {
                    return Err(bad("to must be non-negative"));
                }
                if s.media.queue.current().is_none() {
                    return Err(bad("no current media item"));
                }
                let duration = s.media.duration();
                let to = if relative {
                    s.media.position + value
                } else {
                    value
                };
                let to = to.max(0.0).min(duration.unwrap_or(f64::MAX));
                if duration.is_some_and(|duration| to >= duration) {
                    let playing = s.media.playing;
                    if s.media.queue.next_item().is_some() {
                        s.media.reset_item();
                        if playing {
                            return s.media_resume();
                        }
                    } else {
                        s.media_stop();
                    }
                    return Ok(());
                }
                if let Some(player) = &s.media.player {
                    player.seek(to)?;
                }
                if let Some(video) = &mut s.media.video {
                    video.seek(to);
                }
                s.media.position = to;
                Ok(())
            },
        },
        CommandSpec {
            id: "media.volume",
            label: "Volume",
            menu: &["Media", "Volume"],
            shortcut: None,
            params: "{value: number}: volume from 0 to 100",
            enabled: |_| true,
            run: |s, p| {
                let value = map(p)?
                    .get("value")
                    .and_then(Params::as_f64)
                    .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
                    .ok_or_else(|| bad("value must be a volume from 0 to 100"))?
                    as f32;
                s.media.gain.set_volume(value)?;
                s.media.volume = value;
                Ok(())
            },
        },
        CommandSpec {
            id: "media.mute",
            label: "Mute",
            menu: &["Media", "Mute"],
            shortcut: None,
            params: "{on?: bool}: toggle or set mute",
            enabled: |_| true,
            run: |s, p| {
                let on = on(p, s.media.muted)?;
                s.media.gain.set_muted(on);
                s.media.muted = on;
                Ok(())
            },
        },
        CommandSpec {
            id: "media.remove",
            label: "Remove from Queue",
            menu: &["Media", "Remove"],
            shortcut: None,
            params: "{index: number}",
            enabled: |s| !s.media.queue.items().is_empty(),
            run: |s, p| {
                let index = index(map(p)?, "index")?;
                s.media_remove_item(index)
            },
        },
        CommandSpec {
            id: "media.move",
            label: "Move Queue Item",
            menu: &["Media", "Move"],
            shortcut: None,
            params: "{from: number, to: number}",
            enabled: |s| s.media.queue.items().len() > 1,
            run: |s, p| {
                let map = map(p)?;
                let from = index(map, "from")?;
                let to = index(map, "to")?;
                s.media.queue.move_item(from, to)?;
                for state in s.layout.states.values_mut() {
                    if state.kind == PaneKind::Media {
                        state.media.selected = state.media.selected.map(|selected| {
                            if selected == from {
                                to
                            } else if from < selected && selected <= to {
                                selected - 1
                            } else if to <= selected && selected < from {
                                selected + 1
                            } else {
                                selected
                            }
                        });
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.clear",
            label: "Clear Queue",
            menu: &["Media", "Clear queue"],
            shortcut: None,
            params: "none",
            enabled: |s| !s.media.queue.items().is_empty(),
            run: |s, p| {
                no_params(p)?;
                s.media_stop();
                s.media.queue.clear();
                s.media.reset_item();
                for state in s.layout.states.values_mut() {
                    if state.kind == PaneKind::Media {
                        state.media = Default::default();
                    }
                }
                Ok(())
            },
        },
        CommandSpec {
            id: "media.shuffle",
            label: "Shuffle",
            menu: &["Media", "Shuffle"],
            shortcut: None,
            params: "{on?: bool}: toggle or set shuffle",
            enabled: |_| true,
            run: |s, p| {
                let on = on(p, s.media.queue.shuffle())?;
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|_| bad("system clock is before the Unix epoch"))?
                    .as_nanos() as u64;
                s.media.queue.set_shuffle(on, seed);
                Ok(())
            },
        },
        CommandSpec {
            id: "media.repeat",
            label: "Repeat",
            menu: &["Media", "Repeat"],
            shortcut: None,
            params: "{mode: \"off\"|\"all\"|\"one\"}",
            enabled: |_| true,
            run: |s, p| {
                let mode = match map(p)?.get("mode").and_then(Params::as_str) {
                    Some("off") => RepeatMode::Off,
                    Some("all") => RepeatMode::All,
                    Some("one") => RepeatMode::One,
                    _ => return Err(bad("mode must be off, all or one")),
                };
                s.media.queue.set_repeat(mode);
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn queued(count: usize) -> Session {
        let mut s = Session::new().expect("session");
        let directory = tempfile::tempdir().expect("local test directory");
        for index in 0..count {
            let path = directory.path().join(format!("{index}.mp3"));
            s.media
                .queue
                .enqueue(VfsUri::parse(path.to_str().expect("UTF-8 test path")).expect("uri"))
                .expect("enqueue");
        }
        s.media.queue.select(0).expect("current");
        s.media.playing = true;
        s.audio_owner = AudioOwner::Media;
        s
    }

    fn status(state: PlaybackState) -> PlaybackStatus {
        PlaybackStatus {
            token: 0,
            state,
            info: None,
            position: 1.0,
            error: (state == PlaybackState::Failed).then(|| "decoder failed".into()),
        }
    }

    #[test]
    fn a_folder_stops_at_its_end_rewound_and_manual_next_still_leaves_it() {
        let mut s = queued(4);
        s.media.queue.group_range(1, 2, "f", None).expect("group");
        s.media.queue.select(1).expect("select");
        s.media_apply_status(status(PlaybackState::Finished));
        assert_eq!(s.media.queue.current_index(), Some(2), "inside: advances");
        assert!(s.media.playing);
        s.media_apply_status(status(PlaybackState::Finished));
        assert_eq!(
            s.media.queue.current_index(),
            Some(1),
            "rewound to folder start"
        );
        assert!(!s.media.playing);
        s.media.queue.select(2).expect("select");
        assert!(s.media.queue.next_item().is_some());
        assert_eq!(s.media.queue.current_index(), Some(3));
    }

    #[test]
    fn completion_advances_and_repeat_modes_work() {
        for (repeat, start, expected, playing) in [
            (RepeatMode::Off, 0, 1, true),
            (RepeatMode::Off, 1, 1, false),
            (RepeatMode::All, 1, 0, true),
            (RepeatMode::One, 1, 1, true),
        ] {
            let mut s = queued(2);
            s.media.queue.select(start).expect("select");
            s.media.queue.set_repeat(repeat);
            s.media_apply_status(status(PlaybackState::Finished));
            assert_eq!(s.media.queue.current_index(), Some(expected));
            assert_eq!(s.media.playing, playing);
            assert_eq!(
                s.audio_owner,
                if playing {
                    AudioOwner::Media
                } else {
                    AudioOwner::None
                }
            );
        }
    }

    #[test]
    fn failed_items_skip_repeat_one_and_stop_after_one_failed_cycle() {
        for repeat in [RepeatMode::One, RepeatMode::All] {
            let mut s = queued(2);
            s.media.queue.set_repeat(repeat);
            s.media_apply_status(status(PlaybackState::Failed));
            assert_eq!(s.media.queue.current_index(), Some(1));
            assert!(s.media.playing);
            s.media_apply_status(status(PlaybackState::Failed));
            assert!(!s.media.playing);
            assert_eq!(s.audio_owner, AudioOwner::None);
            assert_eq!(s.messages.len(), 2);
            assert!(
                s.messages
                    .recent()
                    .all(|message| message.text.contains(".mp3"))
            );
        }
    }

    #[test]
    fn preview_and_media_take_ownership_in_both_directions() {
        let mut s = queued(1);
        let pane = s.layout.focused();
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("preview");
        let mut slot = crate::preview_cmds::PreviewSlot::new();
        slot.body = crate::preview_cmds::Body::Static(Arc::new(fm_preview::Preview::Meta(
            fm_preview::MetaDoc {
                rows: vec![],
                hint: None,
                poster: None,
                video: None,
                audio: Some(fm_preview::audio::AudioDoc {
                    path: "/nonexistent-fm-media-tests/preview.mp3".into(),
                    duration: Some(60.0),
                }),
            },
        )));
        s.previews.slots.insert(pane, slot);
        s.audio_preview_tick();
        s.dispatch("preview.audio.play", &json!({"pane": pane, "on": true}))
            .expect("preview play");
        assert!(!s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::PreviewAudio(pane));
        s.dispatch("media.toggle", &Params::Null)
            .expect("media resume");
        assert!(
            !s.previews.slots[&pane]
                .audio
                .as_ref()
                .expect("audio")
                .playing
        );
        assert!(s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::Media);

        s.claim_audio(AudioOwner::PreviewVideo(pane));
        assert!(!s.media.playing);
        s.media_apply_status(status(PlaybackState::Finished));
        assert!(
            !s.media.playing,
            "late completion cannot take ownership back"
        );
        assert_eq!(s.audio_owner, AudioOwner::PreviewVideo(pane));
    }

    #[test]
    fn closing_or_switching_media_respects_background_preference_and_keeps_queue() {
        for background in [false, true] {
            for close in [false, true] {
                let mut s = queued(2);
                s.prefs.media_background_audio = background;
                let first = s.layout.focused();
                let media = s.layout.split(first, Axis::Horizontal, 0.5).expect("split");
                s.dispatch("pane.kind.set", &json!({"kind": "media", "pane": media}))
                    .expect("media");
                if close {
                    s.dispatch("pane.close", &json!({"pane": media}))
                        .expect("close");
                } else {
                    s.dispatch("pane.kind.set", &json!({"kind": "files", "pane": media}))
                        .expect("files");
                }
                assert_eq!(s.media.playing, background);
                assert_eq!(s.media.queue.items().len(), 2);
            }
        }
    }

    #[test]
    fn enqueue_creates_one_media_pane_and_preserves_files_selection_without_autoplay() {
        let mut s = Session::new().expect("session");
        let files = s.layout.focused();
        s.layout
            .state_mut(files)
            .expect("files")
            .listing
            .select_only(Some("song.mp3"), 0);
        s.dispatch("media.enqueue", &json!({"uris": ["/music/song.mp3"]}))
            .expect("enqueue");
        let media = s.media_pane().expect("media pane");
        assert_ne!(files, media);
        assert_eq!(
            s.layout
                .state(files)
                .expect("files")
                .listing
                .selected_count(),
            1
        );
        assert!(!s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::None);
        s.dispatch("media.enqueue", &json!({"uris": ["/music/another.mp3"]}))
            .expect("enqueue");
        assert_eq!(s.layout.panes().len(), 2);
        s.dispatch("pane.split.horizontal", &Params::Null)
            .expect("split media");
        assert_eq!(
            s.layout
                .states
                .values()
                .filter(|state| state.kind == PaneKind::Media)
                .count(),
            1
        );
    }

    #[test]
    fn malformed_commands_do_not_mutate_playback() {
        let mut s = queued(2);
        for (id, params) in [
            ("media.play", json!({"index": -1})),
            ("media.play", json!({"index": 10})),
            (
                "media.play",
                json!({"index": 0, "uris": ["/music/song.mp3"]}),
            ),
            ("media.play", json!({"unknown": true})),
            (
                "media.enqueue",
                json!({"uris": ["/music/song.mp3", "http://host/song.mp3"]}),
            ),
            (
                "media.enqueue",
                json!({"uris": ["sftp://secret:password@host/song.mp3"]}),
            ),
            ("media.pause", json!(1)),
            ("media.toggle", json!({"on": true})),
            ("media.stop", json!({"unknown": true})),
            ("media.next", json!({"index": 0})),
            ("media.prev", json!({"index": 0})),
            ("media.seek", json!({"to": -1})),
            ("media.seek", json!({"to": 1, "by": 1})),
            ("media.seek", json!({})),
            ("media.volume", json!({"value": 101})),
            ("media.mute", json!({"on": "yes"})),
            ("media.remove", json!({"index": 2})),
            ("media.move", json!({"from": 0, "to": 3})),
            ("media.clear", json!(false)),
            ("media.shuffle", json!({"on": 1})),
            ("media.repeat", json!({"mode": "bad"})),
        ] {
            let before = s.media.queue.clone();
            let error = s.dispatch(id, &params).expect_err(id).to_string();
            assert!(!error.contains("secret") && !error.contains("password"));
            assert_eq!(s.media.queue, before, "{id}");
            assert!(s.media.playing, "{id}");
            assert_eq!(s.audio_owner, AudioOwner::Media, "{id}");
        }
    }

    #[test]
    fn seeks_clamp_and_advance_at_the_end_and_volume_and_mute_validate() {
        let mut s = queued(2);
        s.media.info = Some(Arc::new(AudioInfo {
            encoder_delay: 0,
            encoder_padding: 0,
            format: fm_media::AudioFormat::new(48_000, 2).expect("format"),
            duration: Some(10.0),
            tags: Default::default(),
        }));
        s.dispatch("media.seek", &json!({"by": -15}))
            .expect("clamp");
        assert_eq!(s.media.position, 0.0);
        s.dispatch("media.seek", &json!({"to": 50}))
            .expect("advance");
        assert_eq!(s.media.queue.current_index(), Some(1));
        s.dispatch("media.volume", &json!({"value": 25}))
            .expect("volume");
        s.dispatch("media.mute", &json!({"on": true}))
            .expect("mute");
        assert_eq!(s.media.volume, 25.0);
        assert!(s.media.muted);
    }

    #[test]
    fn background_preference_defaults_persists_and_stops_hidden_audio_when_disabled() {
        let mut s = queued(1);
        assert!(s.prefs.media_background_audio);
        s.dispatch(
            "prefs.set",
            &json!({"key": "media_background_audio", "value": false}),
        )
        .expect("preference");
        assert!(!s.media.playing);
        let saved = toml::to_string(&s.prefs).expect("save");
        let loaded: crate::prefs::Prefs = toml::from_str(&saved).expect("load");
        assert!(!loaded.media_background_audio);
        assert!(
            toml::from_str::<crate::prefs::Prefs>("")
                .expect("old prefs")
                .media_background_audio
        );
    }

    #[test]
    fn queue_edits_preserve_current_item_and_remap_view_selection() {
        let mut s = queued(3);
        s.media.playing = false;
        s.dispatch("pane.kind.set", &json!({"kind": "media"}))
            .expect("media");
        let pane = s.media_pane().expect("pane");
        s.layout.state_mut(pane).expect("state").media.selected = Some(0);
        let current = s.media.queue.current().cloned();
        s.dispatch("media.move", &json!({"from": 0, "to": 2}))
            .expect("move");
        assert_eq!(s.media.queue.current(), current.as_ref());
        assert_eq!(s.layout.state(pane).expect("state").media.selected, Some(2));
        s.dispatch("media.remove", &json!({"index": 0}))
            .expect("remove other");
        assert_eq!(s.media.queue.current(), current.as_ref());
        assert_eq!(s.layout.state(pane).expect("state").media.selected, Some(1));
        s.dispatch("media.remove", &json!({"index": 1}))
            .expect("remove selected");
        assert_eq!(s.layout.state(pane).expect("state").media.selected, None);
        assert!(!s.media.playing);
        s.dispatch("media.clear", &Params::Null).expect("clear");
        assert!(s.media.queue.items().is_empty());
        assert_eq!(s.layout.state(pane).expect("state").media.queue_scroll, 0.0);
    }

    #[test]
    fn missing_files_advance_on_worker_failure_without_spinning_in_repeat_all() {
        let mut s = queued(2);
        s.media.queue.set_repeat(RepeatMode::All);
        s.media_resume().expect("start worker");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while s.media.playing {
            s.media_tick();
            assert!(
                std::time::Instant::now() < deadline,
                "failed queue did not stop"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.audio_owner, AudioOwner::None);
        assert!(
            s.media
                .error
                .as_deref()
                .is_some_and(|error| error.contains("1.mp3"))
        );
    }

    #[test]
    fn cover_decodes_off_thread_and_a_replaced_items_art_is_discarded() {
        let mut s = queued(1);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(16, 8)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("png");
        let info = Arc::new(AudioInfo {
            encoder_delay: 0,
            encoder_padding: 0,
            format: fm_media::AudioFormat::new(48_000, 2).expect("format"),
            duration: Some(60.0),
            tags: fm_media::AudioTags {
                cover: Some(fm_media::CoverArt {
                    media_type: "image/png".into(),
                    bytes: bytes.into_inner(),
                }),
                ..Default::default()
            },
        });
        s.media_apply_status(PlaybackStatus {
            info: Some(info.clone()),
            ..status(PlaybackState::Playing)
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while s.media.cover.is_none() {
            s.media_tick();
            assert!(std::time::Instant::now() < deadline, "cover did not load");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(
            s.media
                .cover
                .as_ref()
                .map(|cover| (cover.width, cover.height)),
            Some((16, 8))
        );
        // The OS media controls get our own re-encoded PNG, never the embedded bytes.
        let file = s.media.cover_file.clone().expect("cover file");
        let png = image::open(&file).expect("png");
        assert_eq!((png.width(), png.height()), (16, 8));
        s.media.reset_item();
        s.media_apply_status(PlaybackStatus {
            info: Some(info),
            ..status(PlaybackState::Playing)
        });
        s.media.reset_item();
        s.media_tick();
        assert!(
            s.media.cover.is_none() && s.media.cover_file.is_none(),
            "old artwork must not appear on a new item"
        );
        let _ = std::fs::remove_file(file);
    }
}
