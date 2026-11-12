//! The operating system's media controls: MPRIS on Linux, System Media Transport Controls on
//! Windows, Now Playing on macOS (through `souvlaki`). Hardware media keys and the system
//! overlay drive the Media pane, and the overlay shows what plays.
//!
//! Best effort, as the `media-integration` spec asks: the controls register when playback
//! starts and go away when it ends; a failure is logged once and never affects playback.
//! Events arrive on souvlaki's own thread, are queued, and are dispatched on the UI
//! thread, only as `media.*` commands. Requests to open a URI, raise or quit are ignored.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use fm_engine::Session;
use serde_json::{Value, json};
use souvlaki::{MediaControlEvent, MediaMetadata, MediaPlayback, MediaPosition, SeekDirection};

/// How far an unspecified "seek" from the OS moves.
const SEEK_STEP: f64 = 10.0;
/// Longest seek accepted from the OS in one request.
const MAX_SEEK: f64 = 24.0 * 3600.0;
/// While playing, the position is re-sent this often so overlays stay in step.
const POSITION_EVERY: Duration = Duration::from_secs(1);

/// What the controls show about the current item.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
    pub cover_url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    pub playing: bool,
    pub position: f64,
}

/// One OS integration. `souvlaki` in the app, a recorder in tests.
pub trait Backend {
    fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String>;
    fn set_playback(&mut self, progress: Progress) -> Result<(), String>;
}

/// Connects a backend whose events are sent to `events`.
pub type Connect<'a> =
    dyn FnMut(Sender<MediaControlEvent>) -> Result<Box<dyn Backend>, String> + 'a;

/// The command for an OS control event; `None` for requests the app does not honour.
pub fn command_for(event: &MediaControlEvent, playing: bool) -> Option<(&'static str, Value)> {
    let seek = |direction: SeekDirection, by: f64| {
        let by = by.clamp(0.0, MAX_SEEK);
        let by = if direction == SeekDirection::Backward {
            -by
        } else {
            by
        };
        ("media.seek", json!({ "by": by }))
    };
    Some(match event {
        // `media.play` would play a Files selection; resuming is what the key means here.
        MediaControlEvent::Play if playing => return None,
        MediaControlEvent::Play | MediaControlEvent::Toggle => ("media.toggle", Value::Null),
        MediaControlEvent::Pause if !playing => return None,
        MediaControlEvent::Pause => ("media.pause", Value::Null),
        MediaControlEvent::Stop => ("media.stop", Value::Null),
        MediaControlEvent::Next => ("media.next", Value::Null),
        MediaControlEvent::Previous => ("media.prev", Value::Null),
        MediaControlEvent::Seek(direction) => seek(*direction, SEEK_STEP),
        MediaControlEvent::SeekBy(direction, by) => seek(*direction, by.as_secs_f64()),
        MediaControlEvent::SetPosition(MediaPosition(at)) => (
            "media.seek",
            json!({ "to": at.as_secs_f64().clamp(0.0, MAX_SEEK) }),
        ),
        MediaControlEvent::SetVolume(volume) if volume.is_finite() => (
            "media.volume",
            json!({ "value": (volume * 100.0).clamp(0.0, 100.0) }),
        ),
        // Opening files or URLs, raising, quitting and anything newer stay with the user.
        _ => return None,
    })
}

/// The current item and its progress, or `None` when nothing is playing or paused mid-item.
pub fn now_playing(s: &Session) -> Option<(NowPlaying, Progress)> {
    let media = &s.media;
    let current = media.queue.current()?;
    let position = media
        .video
        .as_ref()
        .map_or(media.position, |video| video.pos);
    if !media.playing && position <= 0.0 {
        return None;
    }
    let tags = media.info.as_ref().map(|info| &info.tags);
    let title = tags
        .and_then(|tags| tags.title.clone())
        .or_else(|| current.file_name().map(str::to_owned))
        .unwrap_or_default();
    let now = NowPlaying {
        title,
        artist: tags.and_then(|tags| tags.artist.clone()),
        album: tags.and_then(|tags| tags.album.clone()),
        duration: media.duration(),
        cover_url: media.cover_file.as_deref().and_then(file_url),
    };
    let progress = Progress {
        playing: media.playing,
        position: position.max(0.0),
    };
    Some((now, progress))
}

/// A `file://` URL for a local path, percent-encoding everything but unreserved bytes.
pub fn file_url(path: &std::path::Path) -> Option<String> {
    let text = path.to_str()?;
    let slashed = text.replace('\\', "/");
    let mut url = String::from("file://");
    if !slashed.starts_with('/') {
        url.push('/');
    }
    for byte in slashed.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                url.push(char::from(byte));
            }
            b':' if url.len() <= "file:///".len() + 1 => url.push(':'),
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    Some(url)
}

/// Keeps one backend in step with the session and turns its events into commands.
pub struct MediaControls {
    backend: Option<Box<dyn Backend>>,
    events: Option<Receiver<MediaControlEvent>>,
    /// Registration failed for this playback; retried once playback has ended.
    failed: bool,
    sent_now: Option<NowPlaying>,
    sent_progress: Option<(Progress, Instant)>,
}

impl MediaControls {
    pub fn new() -> Self {
        Self {
            backend: None,
            events: None,
            failed: false,
            sent_now: None,
            sent_progress: None,
        }
    }

    #[cfg(test)]
    pub fn is_registered(&self) -> bool {
        self.backend.is_some()
    }

    /// Once per frame: dispatch queued OS events, then register, update or remove.
    pub fn tick(&mut self, s: &mut Session, connect: &mut Connect<'_>) -> Vec<&'static str> {
        let mut ran = Vec::new();
        if let Some(events) = &self.events {
            let pending: Vec<_> = events.try_iter().take(64).collect();
            for event in pending {
                if let Some((id, params)) = command_for(&event, s.media.playing)
                    && s.command_enabled(id)
                    && s.dispatch(id, &params).is_ok()
                {
                    ran.push(id);
                }
            }
        }
        let Some((now, progress)) = now_playing(s) else {
            // Playback ended: unregister, and allow a fresh attempt next time.
            self.backend = None;
            self.events = None;
            self.failed = false;
            self.sent_now = None;
            self.sent_progress = None;
            return ran;
        };
        if self.backend.is_none() && !self.failed {
            let (tx, rx) = channel();
            match connect(tx) {
                Ok(backend) => {
                    self.backend = Some(backend);
                    self.events = Some(rx);
                }
                Err(error) => {
                    self.failed = true;
                    eprintln!("media controls unavailable: {error}");
                }
            }
        }
        let Some(backend) = self.backend.as_mut() else {
            return ran;
        };
        if self.sent_now.as_ref() != Some(&now) {
            if let Err(error) = backend.set_metadata(&now) {
                eprintln!("media controls: {error}");
            }
            self.sent_now = Some(now);
            self.sent_progress = None;
        }
        let due = match self.sent_progress {
            None => true,
            Some((sent, at)) => {
                sent.playing != progress.playing
                    || (progress.playing && at.elapsed() >= POSITION_EVERY)
                    || (!progress.playing && (sent.position - progress.position).abs() > 0.5)
            }
        };
        if due {
            if let Err(error) = backend.set_playback(progress) {
                eprintln!("media controls: {error}");
            }
            self.sent_progress = Some((progress, Instant::now()));
        }
        ran
    }
}

/// The real backend.
pub struct Souvlaki(souvlaki::MediaControls);

impl Souvlaki {
    /// Register with the OS. `hwnd` is the main window (needed on Windows only). Every
    /// event wakes the UI, so keys work while the window sits idle in the background.
    pub fn connect(
        hwnd: Option<*mut std::ffi::c_void>,
        ctx: egui::Context,
        events: Sender<MediaControlEvent>,
    ) -> Result<Box<dyn Backend>, String> {
        let config = souvlaki::PlatformConfig {
            display_name: fm_brand::APP_NAME,
            dbus_name: fm_brand::APP_SLUG,
            hwnd,
        };
        let mut controls =
            souvlaki::MediaControls::new(config).map_err(|error| format!("{error:?}"))?;
        controls
            .attach(move |event| {
                let _ = events.send(event);
                ctx.request_repaint();
            })
            .map_err(|error| format!("{error:?}"))?;
        Ok(Box::new(Self(controls)))
    }
}

impl Backend for Souvlaki {
    fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String> {
        self.0
            .set_metadata(MediaMetadata {
                title: Some(&now.title),
                artist: now.artist.as_deref(),
                album: now.album.as_deref(),
                cover_url: now.cover_url.as_deref(),
                duration: now
                    .duration
                    .filter(|d| d.is_finite() && *d > 0.0)
                    .map(Duration::from_secs_f64),
            })
            .map_err(|error| format!("{error:?}"))
    }

    fn set_playback(&mut self, progress: Progress) -> Result<(), String> {
        let at = Some(MediaPosition(Duration::from_secs_f64(
            progress.position.clamp(0.0, MAX_SEEK),
        )));
        self.0
            .set_playback(if progress.playing {
                MediaPlayback::Playing { progress: at }
            } else {
                MediaPlayback::Paused { progress: at }
            })
            .map_err(|error| format!("{error:?}"))
    }
}

/// The native window for SMTC, which needs it to register on Windows.
#[cfg(windows)]
pub fn window_handle(frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(windows))]
pub fn window_handle(_frame: &eframe::Frame) -> Option<*mut std::ffi::c_void> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    type Log = Rc<RefCell<Vec<String>>>;

    struct Fake(Log);

    impl Backend for Fake {
        fn set_metadata(&mut self, now: &NowPlaying) -> Result<(), String> {
            self.0.borrow_mut().push(format!("meta {}", now.title));
            Ok(())
        }
        fn set_playback(&mut self, progress: Progress) -> Result<(), String> {
            let state = if progress.playing {
                "playing"
            } else {
                "paused"
            };
            self.0
                .borrow_mut()
                .push(format!("{state} {:.0}", progress.position));
            Ok(())
        }
    }

    fn session() -> Session {
        let mut s = Session::new().expect("session");
        s.dispatch(
            "media.enqueue",
            &json!({"uris": ["/music/First.mp3", "/music/Second.mp3"]}),
        )
        .expect("enqueue");
        s.media.queue.select(0).expect("current");
        s
    }

    #[test]
    fn os_events_map_to_media_commands_only() {
        use MediaControlEvent as E;
        assert_eq!(
            command_for(&E::Play, false).map(|c| c.0),
            Some("media.toggle")
        );
        assert_eq!(command_for(&E::Play, true), None, "already playing");
        assert_eq!(
            command_for(&E::Pause, true).map(|c| c.0),
            Some("media.pause")
        );
        assert_eq!(command_for(&E::Pause, false), None, "already paused");
        assert_eq!(
            command_for(&E::Toggle, true).map(|c| c.0),
            Some("media.toggle")
        );
        assert_eq!(command_for(&E::Stop, true).map(|c| c.0), Some("media.stop"));
        assert_eq!(command_for(&E::Next, true).map(|c| c.0), Some("media.next"));
        assert_eq!(
            command_for(&E::Previous, true).map(|c| c.0),
            Some("media.prev")
        );
        assert_eq!(
            command_for(&E::Seek(SeekDirection::Backward), true),
            Some(("media.seek", json!({"by": -10.0})))
        );
        assert_eq!(
            command_for(
                &E::SeekBy(SeekDirection::Forward, Duration::from_secs(u64::MAX / 4)),
                true
            ),
            Some(("media.seek", json!({"by": MAX_SEEK})))
        );
        assert_eq!(
            command_for(
                &E::SetPosition(MediaPosition(Duration::from_secs(42))),
                true
            ),
            Some(("media.seek", json!({"to": 42.0})))
        );
        assert_eq!(
            command_for(&E::SetVolume(0.5), true),
            Some(("media.volume", json!({"value": 50.0})))
        );
        assert_eq!(
            command_for(&E::SetVolume(7.0), true),
            Some(("media.volume", json!({"value": 100.0})))
        );
        assert_eq!(command_for(&E::SetVolume(f64::NAN), true), None);
        // Other programs must not open files, raise or quit the app through the controls.
        for event in [
            E::OpenUri("file:///etc/passwd".into()),
            E::OpenUri("https://example.test/x.mp3".into()),
            E::Raise,
            E::Quit,
            E::SetShuffle(true),
        ] {
            assert_eq!(command_for(&event, true), None, "{event:?}");
        }
    }

    #[test]
    fn controls_register_while_media_plays_and_leave_when_it_stops() {
        let mut s = session();
        let log: Log = Rc::default();
        let events: Rc<RefCell<Option<Sender<MediaControlEvent>>>> = Rc::default();
        let mut controls = MediaControls::new();
        let (l, e) = (log.clone(), events.clone());
        let mut connect = move |tx: Sender<MediaControlEvent>| -> Result<Box<dyn Backend>, String> {
            *e.borrow_mut() = Some(tx);
            Ok(Box::new(Fake(l.clone())))
        };
        controls.tick(&mut s, &mut connect);
        assert!(!controls.is_registered(), "nothing plays yet");

        s.media.position = 5.0;
        controls.tick(&mut s, &mut connect);
        assert!(controls.is_registered());
        assert_eq!(*log.borrow(), ["meta First.mp3", "paused 5"]);
        controls.tick(&mut s, &mut connect);
        assert_eq!(log.borrow().len(), 2, "an unchanged pause sends nothing");

        let tx = events.borrow().clone().expect("events");
        tx.send(MediaControlEvent::OpenUri("file:///etc/passwd".into()))
            .expect("send");
        tx.send(MediaControlEvent::Stop).expect("send");
        let ran = controls.tick(&mut s, &mut connect);
        assert_eq!(ran, ["media.stop"], "the URI request is dropped");
        assert!(!controls.is_registered(), "stopped playback unregisters");
        assert_eq!(s.media.queue.items().len(), 2, "nothing was opened");
    }

    #[test]
    fn a_failed_registration_is_not_retried_until_playback_restarts() {
        let mut s = session();
        s.media.position = 3.0;
        let attempts = Rc::new(RefCell::new(0));
        let counter = attempts.clone();
        let mut connect =
            move |_tx: Sender<MediaControlEvent>| -> Result<Box<dyn Backend>, String> {
                *counter.borrow_mut() += 1;
                Err("no session bus".into())
            };
        let mut controls = MediaControls::new();
        for _ in 0..5 {
            controls.tick(&mut s, &mut connect);
        }
        assert_eq!(*attempts.borrow(), 1, "logged once, not every frame");
        assert!(
            s.media.queue.current().is_some(),
            "playback state is untouched"
        );
        s.media.position = 0.0;
        controls.tick(&mut s, &mut connect);
        s.media.position = 1.0;
        controls.tick(&mut s, &mut connect);
        assert_eq!(*attempts.borrow(), 2, "a new playback tries again");
    }

    #[test]
    fn cover_paths_become_encoded_file_urls() {
        assert_eq!(
            file_url(std::path::Path::new("/tmp/fm-open-1/cover #1 é.png")).as_deref(),
            Some("file:///tmp/fm-open-1/cover%20%231%20%C3%A9.png")
        );
        assert_eq!(
            file_url(std::path::Path::new("C:\\Users\\a b\\cover-0.png")).as_deref(),
            Some("file:///C:/Users/a%20b/cover-0.png")
        );
        assert_eq!(
            file_url(std::path::Path::new("/odd:name.png")).as_deref(),
            Some("file:///odd%3Aname.png")
        );
    }
}
