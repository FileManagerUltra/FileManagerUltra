## MODIFIED Requirements

### Requirement: Video playback is owned by the pane
A Preview pane SHALL play at most one video, with sound, starting automatically unless disabled in Preferences, controllable with play/pause, seek, volume and mute, decoded at a size fitted to the pane; leaving the file, switching the pane kind or closing the pane SHALL stop it and end the decoder process. This simple single-file playback SHALL remain independent of the Media pane's queue and SHALL NOT require a Media pane. Without a decoder available the pane SHALL show metadata and a hint instead; without an audio device it SHALL play silently with a hint.

#### Scenario: Pane closed while playing
- **WHEN** a Preview pane is closed during playback
- **THEN** the decoder child process no longer exists

#### Scenario: Video with sound
- **WHEN** a video with an audio track is previewed on a machine with an audio device
- **THEN** it plays with sound and the controls offer play/pause, seek, volume and mute

#### Scenario: No audio device
- **WHEN** a video is previewed on a machine with no audio device
- **THEN** it plays silently and the pane shows a hint

## ADDED Requirements

### Requirement: Audio playback in Preview
A Preview pane SHALL play one audio file at a time, showing cover art and tags with the same simple controls as video (play/pause, seek, volume, mute), without a queue. Playback SHALL stop when the selection changes, the pane kind switches or the pane closes. Audio SHALL NOT start automatically unless the "autoplay audio" preference is on (off by default) so browsing a music folder stays quiet.

#### Scenario: Preview an mp3
- **WHEN** an mp3 is previewed and the user presses play
- **THEN** it plays with sound and shows its cover art and tags

#### Scenario: Browsing is quiet
- **WHEN** the selection moves through several audio files with autoplay off
- **THEN** nothing plays

#### Scenario: Selection changes
- **WHEN** a previewed audio file is playing and another file is selected
- **THEN** the playback stops

### Requirement: One player at a time and hand-off to the Media pane
Preview playback and the Media pane SHALL share one audio output: starting playback in one SHALL stop playback in the other. Preview SHALL offer "Play in Media pane" and "Add to queue" for audio and video, which stop the Preview playback, continue from the current position and enqueue the file in the Media pane.

#### Scenario: Preview starts while the Media pane plays
- **WHEN** the Media pane is playing and the user starts a video in Preview
- **THEN** the Media pane pauses and the Preview video plays

#### Scenario: Hand-off
- **WHEN** the user chooses "Play in Media pane" on a previewed video at 1:30
- **THEN** the Preview stops and the Media pane plays it from about 1:30
