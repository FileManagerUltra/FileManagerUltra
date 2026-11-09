## ADDED Requirements

### Requirement: Media pane
A pane of kind `Media` SHALL play audio and video with sound from the session playback queue. It SHALL show, for video, the current frame fitted to the pane; for audio, cover art (or a placeholder) with title, artist and album tags; a transport bar with play/pause, seek, elapsed/remaining time, volume, mute, speed, previous, next, shuffle and repeat; and the queue list with the current item highlighted. Colours SHALL come from theme tokens and render correctly in dark and light.

#### Scenario: Play an mp3
- **WHEN** an mp3 is played in a Media pane
- **THEN** audio is output, the cover art and tags are shown, and the elapsed time advances

#### Scenario: Pane is a view
- **WHEN** the Media pane is resized, split or moved
- **THEN** playback position and queue are unchanged

### Requirement: Playback is owned by the session
The playback queue, current item, position, volume, shuffle and repeat state SHALL live in the engine `Session`, not in a pane. At most one item SHALL play at a time across the whole app. Changing the selection in Files or Preview panes SHALL NOT affect playback. Closing or switching away from the Media pane SHALL stop video; audio SHALL continue when "keep playing in background" is on (default) and show a status-bar indicator with a stop control, and SHALL stop when it is off.

#### Scenario: Browse while listening
- **WHEN** music is playing and the user selects other files in a Files pane
- **THEN** the music keeps playing without a gap

#### Scenario: Close the pane during video
- **WHEN** the Media pane is closed while a video plays
- **THEN** the video decoder process no longer exists and playback stops

#### Scenario: Close the pane during audio
- **WHEN** the Media pane is closed while audio plays and background playback is on
- **THEN** audio continues and the status bar shows what is playing

### Requirement: Queue and playlists
The queue SHALL support enqueue, play now, play folder (supported media files of a folder in natural name order), remove, reorder, clear, shuffle and repeat (off, all, one). The player SHALL advance automatically at the end of an item and stop after the last item unless repeat is on. Playlists in M3U, M3U8 and PLS format SHALL be loadable and the queue SHALL be savable as M3U8; relative entries resolve against the playlist's folder. Entries that are missing or unsupported SHALL be skipped with a notice naming them.

#### Scenario: Auto-advance
- **WHEN** a track ends and another follows in the queue
- **THEN** the next track starts

#### Scenario: Repeat all
- **WHEN** the last track ends with repeat set to all
- **THEN** the first track starts

#### Scenario: Shuffle
- **WHEN** shuffle is on
- **THEN** every item plays once per cycle in a random order and previous returns to the item played before

#### Scenario: Missing entry
- **WHEN** a loaded playlist lists a file that does not exist
- **THEN** it is skipped, a notice names it, and the other entries play

#### Scenario: Save and reload
- **WHEN** the queue is saved as M3U8 and loaded again
- **THEN** the same items appear in the same order

### Requirement: Entry points
Files and Preview panes SHALL offer Play, Add to queue and Play folder for supported media (context menu and menu bar); dropping supported files or a folder on a Media pane SHALL enqueue them; a Preview video card SHALL offer "Play in Media pane". If no Media pane exists, Play SHALL switch a suitable pane to `Media` (or open one beside the focused pane) rather than fail.

#### Scenario: Play from a Files pane
- **WHEN** a Files pane has an mp3 selected and `media.play` runs for it with no Media pane open
- **THEN** a Media pane appears and plays it

#### Scenario: Drop on the pane
- **WHEN** three audio files are dropped on a Media pane
- **THEN** they are appended to the queue in order and the first plays if nothing was playing

### Requirement: Transport and tracks
`media.seek` SHALL move to an absolute or relative position clamped to the item's duration; volume SHALL be 0–100 and persist; speed SHALL follow the playback-speed requirement of `media-audio-pipeline`. For video, the pane SHALL list embedded audio and subtitle tracks and let the user choose one or none (subtitles per `media-subtitles`), and SHALL support frame step while paused. Keyboard control SHALL work while the pane is focused: Space play/pause, ←/→ seek, ↑/↓ volume, M mute, N/P next/previous, F fullscreen.

#### Scenario: Seek past the end
- **WHEN** `media.seek` is given a position beyond the duration
- **THEN** it clamps to the end and the next item starts

#### Scenario: Choose an audio track
- **WHEN** a video with two audio tracks plays and the second is chosen
- **THEN** playback continues from the same position with the second track

### Requirement: Audio output and A/V sync
Audio SHALL be the master clock. For video, frames SHALL be dropped when late and held when early so that picture and sound stay in sync; a video without audio SHALL use a monotonic clock. Memory SHALL stay bounded: only the newest few decoded frames and a bounded audio buffer are kept however slow the UI is. If no audio device is available, video SHALL play silently with a hint and audio items SHALL report an error and be skipped.

#### Scenario: Slow UI
- **WHEN** the UI is stalled for two seconds during video playback
- **THEN** playback resumes at the clock position by dropping frames, and memory does not grow

#### Scenario: No audio device
- **WHEN** a video plays on a machine with no audio device
- **THEN** it plays silently and the pane shows a hint

### Requirement: Fullscreen
`media.fullscreen` SHALL toggle a mode in which the Media pane fills the window with all other panes hidden and their state preserved; `window.fullscreen` SHALL toggle native window fullscreen and the two SHALL be usable together. In fullscreen the transport controls and cursor SHALL auto-hide after three seconds without pointer movement and reappear on movement. Esc, F or a double-click SHALL leave fullscreen, restoring the previous layout, focus and window state; fullscreen SHALL NOT overwrite the saved normal window geometry. Playback SHALL continue uninterrupted when entering or leaving fullscreen. Closing the pane or a stopped queue SHALL leave fullscreen.

#### Scenario: Enter and leave
- **WHEN** a video plays and the user presses F, then Esc
- **THEN** the video fills the window, then the previous layout, selection and focus return, with playback never interrupted

#### Scenario: Controls hide
- **WHEN** the pointer is still for three seconds in fullscreen
- **THEN** the controls and cursor hide and reappear on movement

#### Scenario: Not a Media pane
- **WHEN** `media.fullscreen` runs and nothing is playing
- **THEN** it returns an error and the layout is unchanged

### Requirement: Exactly one Media pane
At most one Media pane SHALL exist at a time. Asking for another (`pane.kind.set media`, Play with none focused, a workspace that contains two) SHALL focus the existing one instead; a saved workspace with several SHALL restore the first and turn the rest into Files panes. Preview's player and the Media pane's player SHALL stay separate views of the one-player-at-a-time rule.

#### Scenario: Second Media pane
- **WHEN** a Media pane exists and `pane.kind.set media` runs on another pane
- **THEN** the existing Media pane gets focus and the other pane's kind is unchanged

### Requirement: Remote and unavailable files
A file on a remote backend SHALL be copied through a job to an owner-only temporary spool, with progress shown, before playing; the spool SHALL follow the rules for temporary opened files (size cap, deleted on quit). Without a usable decoder the pane SHALL show metadata and an install hint instead of failing. A file that vanishes or fails to decode SHALL produce an error notice and playback SHALL advance to the next item.

#### Scenario: Play from SFTP
- **WHEN** an mp3 on an SFTP server is played
- **THEN** a job downloads it with progress and it then plays

#### Scenario: Vanished file
- **WHEN** the current file is deleted while queued
- **THEN** a notice names it and the next item plays

### Requirement: Media commands
All media behaviour SHALL be commands with id, label, menu path, shortcut, params documentation and `enabled`: `media.play`, `media.pause`, `media.toggle`, `media.stop`, `media.next`, `media.prev`, `media.seek`, `media.volume`, `media.mute`, `media.speed`, `media.enqueue`, `media.play_folder`, `media.remove`, `media.move`, `media.clear`, `media.shuffle`, `media.repeat`, `media.track.audio`, `media.track.subtitle`, `media.step`, `media.fullscreen`, `media.playlist.load`, `media.playlist.save`, `media.loop`, `media.sleep`, `media.eq`, `media.skip`, `media.chapter`, `media.subtitle.load`, `media.subtitle.delay`, `media.bookmark`, `media.finished` and `media.book`. Bad, oversized or malformed params SHALL return an error and never panic; the command fuzzer SHALL cover them.

#### Scenario: Malformed params
- **WHEN** `media.seek` receives a string, a negative index, NaN or an object without the expected fields
- **THEN** it returns an error and playback is unchanged
