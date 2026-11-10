//! Shared Files/Preview entry points for the non-destructive Media queue actions.

use fm_layout::PaneKind;
use fm_vfs::{FmError, VfsUri};

use crate::Session;
use crate::file_cmds::{Obj, focused_selection, object};
use crate::media_cmds::validate_media_source;
use crate::registry::CommandSpec;

pub(crate) struct MediaInput {
    pub items: Vec<VfsUri>,
    pub position: f64,
    pub preview: Option<fm_layout::PaneId>,
}

fn bad(message: &str) -> FmError {
    FmError::InvalidParams(message.into())
}

pub(crate) fn sources(s: &Session, map: Option<&Obj>) -> Result<MediaInput, FmError> {
    let explicit = map.and_then(|map| map.get("uris"));
    let pane = map.and_then(|map| map.get("pane"));
    if explicit.is_some() && pane.is_some() {
        return Err(bad("choose either uris or a Preview pane"));
    }
    let preview = if let Some(pane) = pane {
        Some(
            pane.as_u64()
                .ok_or_else(|| bad("pane must be a Preview pane id"))?,
        )
    } else {
        let focused = s.layout.focused();
        (explicit.is_none() && s.pane_kind(focused) == Some(PaneKind::Preview)).then_some(focused)
    };
    let mut position = 0.0;
    let items = if let Some(pane) = preview {
        if s.pane_kind(pane) != Some(PaneKind::Preview) {
            return Err(bad("pane must identify a Preview pane"));
        }
        let slot = s
            .previews
            .slots
            .get(&pane)
            .filter(|slot| slot.playback_ready())
            .ok_or_else(|| bad("Preview is not showing playable local media"))?;
        let path = if let Some(audio) = &slot.audio {
            position = audio.position;
            audio
                .doc
                .path
                .to_str()
                .ok_or_else(|| bad("media path must be UTF-8"))?
        } else if let Some(video) = &slot.video {
            position = video.pos;
            video.info.path.as_str()
        } else {
            return Err(bad("Preview is not showing playable local media"));
        };
        if !position.is_finite() || position < 0.0 {
            return Err(bad("invalid Preview position"));
        }
        vec![VfsUri::parse(path).map_err(|_| bad("invalid Preview media path"))?]
    } else if let Some(value) = explicit {
        let values = value
            .as_array()
            .filter(|values| values.len() <= fm_media::MAX_QUEUE_ITEMS)
            .ok_or_else(|| bad("uris must be a bounded array of media paths"))?;
        values
            .iter()
            .map(|value| {
                let raw = value
                    .as_str()
                    .ok_or_else(|| bad("uris must contain strings"))?;
                VfsUri::parse(raw).map_err(|_| bad("invalid media URI"))
            })
            .collect::<Result<Vec<_>, _>>()?
    } else {
        focused_selection(s)
    };
    if items.is_empty() {
        return Err(bad(
            "select media files, focus a playable Preview or supply uris",
        ));
    }
    if s.media.queue.items().len().saturating_add(items.len()) > fm_media::MAX_QUEUE_ITEMS {
        return Err(bad("media queue is full"));
    }
    for uri in &items {
        validate_media_source(uri)?;
    }
    Ok(MediaInput {
        items,
        position,
        preview,
    })
}

pub(crate) fn has_sources(s: &Session) -> bool {
    sources(s, None).is_ok()
}

impl Session {
    pub(crate) fn media_insert_next(&mut self, items: Vec<VfsUri>) -> Result<usize, FmError> {
        let count = items.len();
        let first = self.media.queue.insert_next(items)?;
        for state in self
            .layout
            .states
            .values_mut()
            .filter(|state| state.kind == PaneKind::Media)
        {
            state.media.selected = state
                .media
                .selected
                .map(|index| if index >= first { index + count } else { index });
        }
        Ok(first)
    }

    pub(crate) fn stop_handed_off_preview(&mut self, pane: Option<fm_layout::PaneId>) {
        if let Some(slot) = pane.and_then(|pane| self.previews.slots.get_mut(&pane)) {
            if let Some(audio) = &mut slot.audio {
                audio.stop();
            }
            if let Some(video) = &mut slot.video {
                video.pause();
            }
        }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "media.play_next",
        label: "Play Next",
        menu: &["Media", "Play next"],
        shortcut: None,
        params: "{uris?: string[], pane?: number}: insert after the current item without changing playback; Files selection or Preview by default",
        enabled: has_sources,
        run: |s, params| {
            let map = object(params)?;
            if map.is_some_and(|map| {
                map.keys()
                    .any(|key| !matches!(key.as_str(), "uris" | "pane"))
            }) {
                return Err(bad("only uris or pane is accepted"));
            }
            let input = sources(s, map)?;
            s.show_media_pane()?;
            s.media_insert_next(input.items)?;
            Ok(())
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio_owner::AudioOwner;
    use crate::preview_cmds::{Body, PreviewSlot};
    use crate::registry::Params;
    use fm_preview::{MetaDoc, Preview, audio::AudioDoc};
    use serde_json::json;
    use std::sync::Arc;

    fn setup() -> (Session, fm_layout::PaneId, VfsUri) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("preview.mp3");
        let uri = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
        let mut s = Session::new().expect("session");
        s.dispatch("pane.kind.set", &json!({"kind": "preview"}))
            .expect("preview");
        let pane = s.layout.focused();
        let mut slot = PreviewSlot::new();
        slot.body = Body::Static(Arc::new(Preview::Meta(MetaDoc {
            rows: Vec::new(),
            hint: None,
            poster: None,
            video: None,
            audio: Some(AudioDoc {
                path,
                duration: Some(120.0),
            }),
        })));
        s.previews.slots.insert(pane, slot);
        s.audio_preview_tick();
        let audio = s
            .previews
            .slots
            .get_mut(&pane)
            .expect("slot")
            .audio
            .as_mut()
            .expect("audio");
        audio.position = 90.0;
        audio.playing = true;
        s.audio_owner = AudioOwner::PreviewAudio(pane);
        for name in ["a.mp3", "b.mp3", "c.mp3"] {
            s.media
                .queue
                .enqueue(uri.parent().expect("parent").join(name).expect("uri"))
                .expect("queue");
        }
        s.media.queue.select(1).expect("current B");
        (s, pane, uri)
    }

    #[test]
    fn preview_actions_have_distinct_placement_and_playback_semantics() {
        for id in ["media.enqueue", "media.play_next", "media.play"] {
            let (mut s, pane, uri) = setup();
            let before = s.media.queue.items().to_vec();
            assert!(s.command_enabled(id), "{id} from focused Preview");
            s.dispatch(id, &json!({"pane": pane})).expect("action");
            assert_eq!(s.media.queue.items().len(), 4);
            assert!(
                before
                    .iter()
                    .all(|item| s.media.queue.items().contains(item))
            );
            let index = if id == "media.enqueue" { 3 } else { 2 };
            assert_eq!(s.media.queue.items()[index], uri);
            let audio = s.previews.slots[&pane].audio.as_ref().expect("audio");
            assert_eq!(audio.position, 90.0);
            if id == "media.play" {
                assert_eq!(s.media.queue.current_index(), Some(2));
                assert_eq!(s.media.position, 90.0);
                assert!(s.media.playing);
                assert!(!audio.playing);
                assert_eq!(s.audio_owner, AudioOwner::Media);
                s.media_stop();
            } else {
                assert_eq!(s.media.queue.current_index(), Some(1));
                assert!(!s.media.playing);
                assert!(audio.playing);
                assert_eq!(s.audio_owner, AudioOwner::PreviewAudio(pane));
            }
        }
    }

    #[test]
    fn source_play_inserts_and_existing_queue_rows_play_in_place() {
        let (mut s, _, uri) = setup();
        s.dispatch("media.play", &json!({"uris": [uri.to_string()]}))
            .expect("play");
        assert_eq!(s.media.queue.current_index(), Some(2));
        assert_eq!(
            s.media.position, 0.0,
            "explicit URI is not a Preview handoff"
        );
        assert_eq!(s.media.queue.items().len(), 4);
        s.dispatch("media.play", &json!({"index": 0})).expect("row");
        assert_eq!(
            s.media.queue.items().len(),
            4,
            "queue row play must not duplicate it"
        );
        assert_eq!(s.media.queue.current_index(), Some(0));
        s.media_stop();
    }

    #[test]
    fn queue_selection_tracks_the_same_item_after_insertion() {
        let (mut s, pane, _) = setup();
        s.show_media_pane().expect("media");
        let media = s.media_pane().expect("pane");
        s.layout.state_mut(media).expect("state").media.selected = Some(2);
        s.dispatch("media.play_next", &json!({"pane": pane}))
            .expect("next");
        assert_eq!(
            s.layout.state(media).expect("state").media.selected,
            Some(3)
        );
        assert_eq!(s.media.queue.items()[3].file_name(), Some("c.mp3"));
    }

    #[test]
    fn preview_video_actions_transfer_position_only_for_play_and_stop_its_decoder() {
        for command in ["media.enqueue", "media.play_next", "media.play"] {
            let (mut s, pane, audio_uri) = setup();
            let path = audio_uri.local_path().expect("path").with_extension("mkv");
            let video_uri = VfsUri::parse(path.to_str().expect("utf8")).expect("uri");
            let slot = s.previews.slots.get_mut(&pane).expect("slot");
            slot.audio = None;
            slot.body = Body::Static(Arc::new(Preview::Meta(MetaDoc {
                rows: Vec::new(),
                hint: None,
                poster: None,
                audio: None,
                video: Some(fm_preview::VideoInfo {
                    path: path.to_string_lossy().into_owned(),
                    width: 320,
                    height: 180,
                    fps: 25.0,
                    duration: 120.0,
                    audio: None,
                    audio_delay: 0.0,
                    tracks: Default::default(),
                }),
            })));
            s.video_tick();
            s.previews
                .slots
                .get_mut(&pane)
                .expect("slot")
                .video
                .as_mut()
                .expect("video")
                .pos = 90.0;
            s.audio_owner = AudioOwner::PreviewVideo(pane);
            assert!(s.command_enabled(command));
            s.dispatch(command, &json!({"pane": pane})).expect("action");
            let index = if command == "media.enqueue" { 3 } else { 2 };
            assert_eq!(s.media.queue.items()[index], video_uri);
            assert_eq!(s.media.queue.items().len(), 4);
            let video = s.previews.slots[&pane].video.as_ref().expect("video");
            if command == "media.play" {
                assert!(!video.playing);
                assert!(video.player_is_idle());
                assert_eq!(s.media.position, 90.0);
                assert_eq!(s.media.queue.current_index(), Some(2));
                assert_eq!(s.audio_owner, AudioOwner::Media);
                s.media_stop();
            } else {
                assert!(video.playing);
                assert_eq!(video.pos, 90.0);
                assert_eq!(s.media.queue.current_index(), Some(1));
                assert_eq!(s.audio_owner, AudioOwner::PreviewVideo(pane));
            }
        }
    }

    #[test]
    fn bad_sources_leave_preview_and_queue_unchanged() {
        let (mut s, pane, _) = setup();
        let before = s.media.queue.clone();
        let panes = s.layout.panes().len();
        for command in ["media.play", "media.play_next", "media.enqueue"] {
            for params in [
                json!({"pane": -1}),
                json!({"pane": "preview"}),
                json!({"pane": 999}),
                json!({"uris": ["/song.mp3"], "pane": pane}),
                json!({"uris": ["https://alice:secret@host/song.mp3"]}),
                json!({"uris": []}),
                json!({"unknown": true}),
            ] {
                let error = s.dispatch(command, &params).expect_err("invalid");
                assert!(!error.to_string().contains("secret"));
                assert_eq!(s.media.queue, before);
                assert_eq!(s.layout.panes().len(), panes);
                assert!(
                    s.previews.slots[&pane]
                        .audio
                        .as_ref()
                        .expect("audio")
                        .playing
                );
            }
        }
        assert!(
            s.dispatch("media.play", &json!({"index": 0, "pane": pane}))
                .is_err()
        );
        assert!(
            s.dispatch("media.play_next", &Params::String("bad".into()))
                .is_err()
        );
    }
}
