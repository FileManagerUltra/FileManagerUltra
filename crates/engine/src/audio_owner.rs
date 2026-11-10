//! Session-wide output ownership shared by Preview and Media playback.

use fm_layout::PaneId;

use crate::{Session, preview_audio_cmds::AudioState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AudioOwner {
    #[default]
    None,
    PreviewAudio(PaneId),
    PreviewVideo(PaneId),
    Media,
}

impl Session {
    /// Claim the app-wide output for a player, pausing all other owners first.
    pub fn claim_audio(&mut self, owner: AudioOwner) {
        if self.audio_owner == owner {
            return;
        }
        if owner != AudioOwner::Media {
            self.media.pause();
        }
        for (pane, slot) in &mut self.previews.slots {
            let keep_audio = owner == AudioOwner::PreviewAudio(*pane);
            if !keep_audio && let Some(audio) = &mut slot.audio {
                audio.pause();
            }
            let keep_video = owner == AudioOwner::PreviewVideo(*pane);
            if !keep_video
                && let Some(video) = &mut slot.video
                && video.playing
            {
                video.playing = false;
                video.stop();
            }
        }
        self.audio_owner = owner;
    }

    pub fn release_audio(&mut self, owner: AudioOwner) {
        if self.audio_owner == owner {
            self.audio_owner = AudioOwner::None;
        }
    }

    /// Clear a Preview lease as soon as its item, pane or playback ends.
    pub(crate) fn reconcile_audio_owner(&mut self) {
        let owner_alive = match self.audio_owner {
            AudioOwner::None => true,
            AudioOwner::Media => self.media.playing,
            AudioOwner::PreviewAudio(pane) => self
                .previews
                .slots
                .get(&pane)
                .and_then(|slot| slot.audio.as_ref())
                .is_some_and(|audio: &AudioState| audio.playing),
            AudioOwner::PreviewVideo(pane) => self
                .previews
                .slots
                .get(&pane)
                .and_then(|slot| slot.video.as_ref())
                .is_some_and(|video| video.playing),
        };
        if !owner_alive {
            self.audio_owner = AudioOwner::None;
        }
    }
}
