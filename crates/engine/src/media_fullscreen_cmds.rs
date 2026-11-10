//! Media-pane fullscreen (a view mode of the layout) and native window fullscreen.

use fm_layout::PaneId;
use fm_vfs::FmError;

use crate::{Session, file_cmds::object, registry::CommandSpec};

/// What to put back when Media fullscreen ends. The layout itself is never modified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaFullscreen {
    restore_focus: PaneId,
    restore_zoom: Option<PaneId>,
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

fn can_enter(s: &Session) -> bool {
    s.media_pane().is_some() && s.media.is_video() && (s.media.playing || s.media.video.is_some())
}

fn on(params: &serde_json::Value, current: bool) -> Result<bool, FmError> {
    let map = object(params)?;
    if map.is_some_and(|map| map.keys().any(|key| key != "on")) {
        return Err(bad("only on: bool is accepted"));
    }
    map.and_then(|map| map.get("on"))
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| bad("on must be true or false"))
        })
        .unwrap_or(Ok(!current))
}

impl Session {
    pub fn media_fullscreen_active(&self) -> bool {
        self.media_fullscreen.is_some()
    }

    fn media_fullscreen_enter(&mut self) -> Result<(), FmError> {
        if self.media_fullscreen.is_some() {
            return Ok(());
        }
        if !can_enter(self) {
            return Err(bad("play a video in the Media pane to use fullscreen"));
        }
        let pane = self
            .media_pane()
            .ok_or_else(|| bad("there is no Media pane"))?;
        let restore = MediaFullscreen {
            restore_focus: self.layout.focused(),
            restore_zoom: self.layout.zoomed,
        };
        self.layout.focus_pane(pane)?;
        self.media_fullscreen = Some(restore);
        Ok(())
    }

    /// Leave fullscreen; `restore` also returns focus (and a user zoom) to where they were.
    pub(crate) fn media_fullscreen_leave(&mut self, restore: bool) {
        let Some(saved) = self.media_fullscreen.take() else {
            return;
        };
        if !restore {
            return;
        }
        if self.layout.focus_pane(saved.restore_focus).is_ok()
            && saved.restore_zoom == Some(saved.restore_focus)
            && self.layout.panes().len() > 1
        {
            self.layout.zoomed = saved.restore_zoom;
        }
    }

    /// Fullscreen ends with its pane or when focus leaves it. Called on every poll.
    pub(crate) fn media_fullscreen_check(&mut self) {
        if self.media_fullscreen.is_none() {
            return;
        }
        match self.media_pane() {
            None => self.media_fullscreen_leave(true),
            Some(pane) if self.layout.focused() != pane => self.media_fullscreen_leave(false),
            Some(_) => {}
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "media.fullscreen",
            label: "Media Fullscreen",
            menu: &["Media", "Fullscreen"],
            shortcut: None,
            params: "{on?: bool}: show only the Media pane across the window, or restore the layout",
            enabled: |s| s.media_fullscreen_active() || can_enter(s),
            run: |s, params| {
                let want = on(params, s.media_fullscreen_active())?;
                if want {
                    s.media_fullscreen_enter()
                } else {
                    s.media_fullscreen_leave(true);
                    Ok(())
                }
            },
        },
        CommandSpec {
            id: "window.fullscreen",
            label: "Full Screen",
            menu: &["View", "Full screen"],
            shortcut: Some("F11"),
            params: "{on?: bool}: toggle native window fullscreen; never persisted",
            enabled: |_| true,
            run: |s, params| {
                s.window_fullscreen = on(params, s.window_fullscreen)?;
                Ok(())
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio_owner::AudioOwner, video_cmds::VideoState};
    use serde_json::json;

    fn session() -> Session {
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

    #[test]
    fn fullscreen_round_trip_preserves_layout_focus_and_playback() {
        let mut s = session();
        let files = s
            .layout
            .panes()
            .into_iter()
            .find(|pane| Some(*pane) != s.media_pane())
            .expect("files pane");
        s.layout.focus_pane(files).expect("focus");
        let layout = s.layout.panes();
        s.media.playing = true;
        s.audio_owner = AudioOwner::Media;
        s.dispatch("media.fullscreen", &json!(null)).expect("enter");
        assert!(s.media_fullscreen_active());
        assert_eq!(Some(s.layout.focused()), s.media_pane());
        assert!(s.media.playing);
        s.dispatch("media.fullscreen", &json!(null)).expect("leave");
        assert!(!s.media_fullscreen_active());
        assert_eq!(s.layout.focused(), files);
        assert_eq!(s.layout.panes(), layout);
        assert!(s.media.playing);
        assert_eq!(s.audio_owner, AudioOwner::Media);
    }

    #[test]
    fn nothing_to_show_is_an_error_and_changes_nothing() {
        let mut s = Session::new().expect("session");
        assert!(!s.command_enabled("media.fullscreen"));
        let focus = s.layout.focused();
        assert!(s.dispatch("media.fullscreen", &json!(null)).is_err());
        assert!(!s.media_fullscreen_active());
        assert_eq!(s.layout.focused(), focus);
        let mut s = session();
        s.media.video = None;
        assert!(
            s.dispatch("media.fullscreen", &json!({"on": true}))
                .is_err()
        );
        assert!(!s.media_fullscreen_active());
    }

    #[test]
    fn stop_and_pane_close_leave_fullscreen() {
        let mut s = session();
        s.dispatch("media.fullscreen", &json!({"on": true}))
            .expect("enter");
        s.dispatch("media.stop", &json!(null)).expect("stop");
        assert!(!s.media_fullscreen_active());
        let mut s = session();
        s.dispatch("media.fullscreen", &json!({"on": true}))
            .expect("enter");
        let pane = s.media_pane().expect("pane");
        s.dispatch("pane.close", &json!({"pane": pane}))
            .expect("close");
        assert!(!s.media_fullscreen_active());
    }

    #[test]
    fn window_fullscreen_toggles_and_validates() {
        let mut s = Session::new().expect("session");
        s.dispatch("window.fullscreen", &json!(null)).expect("on");
        assert!(s.window_fullscreen);
        s.dispatch("window.fullscreen", &json!({"on": false}))
            .expect("off");
        assert!(!s.window_fullscreen);
        for params in [json!({"on": "yes"}), json!({"x": 1}), json!(3)] {
            assert!(s.dispatch("window.fullscreen", &params).is_err());
            assert!(!s.window_fullscreen);
        }
    }
}
