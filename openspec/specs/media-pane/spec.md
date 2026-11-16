# media-pane Specification

## Purpose
The Media pane: a player with a persisted playlist for local and remote audio and video, its controls and keyboard use, and how files are opened and played from Files and Preview.
## Requirements
### Requirement: The playlist follows added items
After any request that adds items to the queue (Play, Play next, Add to queue, Play folder, a drop, loading a playlist, or a media file activated with the *Play in Media pane* preference), the Media pane SHALL select exactly the added rows, open a collapsed playlist folder that holds them, and scroll the first added row into view (centred when it was off-screen, no scroll when it is already fully visible). A drop on the Media pane SHALL also give that pane the focus. Adds requested from another pane SHALL leave the focus where it is, because plain arrow keys in the Media pane control volume and seeking. Items skipped as missing or unsupported are not selected. When the Media pane is hidden, the reveal SHALL happen the next time the pane is shown. The engine SHALL expose the added range with a sequence number, and each pane SHALL consume it once.

#### Scenario: Add to a long playlist
- **WHEN** the queue has 300 items, the playlist shows rows 1–20, and the user adds three files to the queue
- **THEN** rows 301–303 are selected, row 301 is scrolled into view, and clicking another row selects only that row

#### Scenario: Play next
- **WHEN** item 5 of 300 is current and the user chooses Play next for two files
- **THEN** rows 6–7 are selected and visible, and playback is unchanged

#### Scenario: Already visible
- **WHEN** the added row is already fully visible
- **THEN** the playlist does not scroll

### Requirement: Drops on the player play, drops on the playlist add
The Media pane SHALL have two drop targets, each highlighted while a drag hovers it. Dropping supported files or folders (from a pane, the OS or the playlist itself) on the **player area** (video picture, audio cover or transport bar) SHALL use Play semantics: insert after the current item and start the first dropped item at once. Dropped files that are already in the playlist SHALL NOT be added again: when the first dropped file is already there, its existing row SHALL be selected, scrolled into view and played (if it is the current item it keeps its place: it resumes when paused and is not restarted when playing); other new files in the same drop are still inserted after the current item. Dropped folders are unaffected. Dropping on the **playlist** SHALL add them at the drop position (before the hovered row when the pointer is on its upper half, after it on its lower half, inside a hovered folder row as now, or at the end below the last row) without changing playback. Nothing starts when nothing is playing, unless the user turned on "Start playing when items are added". The new rows SHALL then be revealed as the playlist-follow requirement says. Drops from other apps SHALL pick the target from the pointer position; when the position is unknown they go to the end of the playlist. `media.drop` SHALL take `play: bool` (default `false`), and `true` SHALL be accepted only without `group`, `after`, `before` or `outside`. Reordering by dragging rows inside the playlist is unchanged.

#### Scenario: Drop on the video
- **WHEN** `B.mkv` is playing in queue A, B, C and the user drops `D.mkv` on the video picture
- **THEN** the queue is A, B, D, C and D starts immediately

#### Scenario: Drop on the playlist while idle
- **WHEN** nothing is playing and the user drops three audio files below the last playlist row
- **THEN** they are appended in order, selected and scrolled into view, and nothing starts playing (with "Start playing when items are added" off, the default)

#### Scenario: Drop between rows
- **WHEN** the user drops `X.mp3` on the playlist between rows 4 and 5
- **THEN** X becomes row 5, is selected, and the current item keeps playing

#### Scenario: Drop on the upper half of a row
- **WHEN** a file or a playlist row is dropped on the upper half of row 3 (outside a locked folder)
- **THEN** it lands directly above row 3, and the drop line shows at the top edge of that row while hovering

#### Scenario: Drop a book folder above another
- **WHEN** an audiobook folder is dropped from a Files pane on the upper part of another book's folder header
- **THEN** it becomes its own playlist folder directly above that book, not at the end of the playlist and never inside the other folder

#### Scenario: Conflicting drop params
- **WHEN** `media.drop` receives `play: true` together with `group` or `after`, or a non-boolean `play`
- **THEN** it returns `Err(InvalidParams)` and the queue is unchanged

#### Scenario: Drop a file that is already in the playlist
- **WHEN** queue A, B, C has A current and the user drops `C` on the player
- **THEN** the queue stays A, B, C, row C is selected and scrolled into view, and C starts playing

#### Scenario: Drop the item that is playing
- **WHEN** `B` is playing at 0:12 and the user drops `B` on the player
- **THEN** nothing is added and B keeps playing from 0:12

### Requirement: Player context menu
Right-clicking the player area (video picture, the now-playing display with the audio cover, or empty space around the transport buttons) SHALL open a context menu holding the player's actions and settings. In this order, with separators between groups: Play/Pause, Stop, Previous, Next; for video, Audio track ▸ (when there is more than one), Subtitles ▸ (Off, embedded and side-loaded tracks, Load subtitle file…, delay) and Subtitle appearance… (opens Preferences on the Media page); Equalizer…, Sleep timer ▸, Loop ▸, and for video Speed ▸; Fullscreen (video only); Show in folder. Each entry SHALL dispatch an existing command by id, show its player key where there is one (Space, N, P, F), be disabled when its command is disabled, and show the current choice (checked track, speed). Entries that do not apply to the current item (video entries for audio, tracks a file lacks) SHALL be hidden. A right-click SHALL NOT toggle playback; single click (pause/play) and double-click (fullscreen) keep their meaning. Shift+F10 SHALL open the menu while the Media pane has focus. Text in the player area is not selectable, so a right-click on it opens the menu.

#### Scenario: Choose a subtitle from the video
- **WHEN** a video with two embedded subtitle tracks plays and the user right-clicks the picture and picks the second track under Subtitles
- **THEN** `media.track.subtitle` is dispatched for it, the menu closes, and playback is not paused

#### Scenario: Audio item
- **WHEN** an mp3 plays and the user right-clicks the cover area
- **THEN** the menu shows transport, Equalizer, Loop, Sleep timer and Show in folder, and no Subtitles or Fullscreen entries

#### Scenario: Nothing loaded
- **WHEN** the queue is empty and the user right-clicks the player area
- **THEN** the menu opens with the transport entries and Show in folder disabled and Equalizer still available

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

"Add to queue" SHALL append to the end without changing playback. "Play next"
SHALL insert the requested files after the current Media item, preserving request
order, without interrupting playback. "Play" SHALL insert there and immediately
start the first requested item. With no current item, Play/Play next SHALL insert
at the front; only Play starts playback. Explicit Play next requests SHALL take
precedence over shuffle, forward navigation history and repeat-one at the next
item boundary. Queue entries SHALL NOT be removed automatically when played,
skipped, finished or interrupted; only explicit remove/clear actions remove them.
Selecting an existing queue row SHALL play that row without inserting a duplicate.
When the current item sits inside a locked playlist folder, Play and Play next
SHALL insert right after that folder instead of joining it; a locked folder never
gains, loses or reorders items.

#### Scenario: Play next while inside a locked folder
- **WHEN** the current item is the second of three items in a locked playlist folder and the user plays a video in Media
- **THEN** the video is inserted directly after the folder, outside it, and starts; the folder still holds its three items

#### Scenario: Three queue actions
- **WHEN** B is current in queue A, B, C and the user adds D to queue, then requests E as Play next
- **THEN** the queue is A, B, E, C, D and B keeps playing

#### Scenario: Play without discarding the playlist
- **WHEN** B is current in queue A, B, C and the user chooses Play for D
- **THEN** the queue is A, B, D, C and D starts immediately, with A, B and C retained

#### Scenario: Play next with shuffle and repeat-one
- **WHEN** shuffle and repeat-one are enabled and the user requests D as Play next
- **THEN** D plays at the next item boundary before ordinary shuffled or repeated playback resumes

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

Playlist rows SHALL be selected with the same rules as the Files listing: click selects one row, Ctrl/Cmd-click toggles a row, Shift-click grows the selection from its existing ends, and Shift+Up/Down/Home/End/PageUp/PageDown extend a range from a fixed anchor (flipping when the cursor crosses it). Plain arrow keys remain playback keys. Removing (context menu or Delete) acts on every selected row.

#### Scenario: Shift+Down in the playlist
- **WHEN** row B is selected and the user presses Shift+Down twice, then Shift+Up three times
- **THEN** B–D are selected, then A–B

DVD-Video folders (a disc folder containing `VIDEO_TS`, or `VIDEO_TS` itself) SHALL be added as one playlist folder named after the disc, holding only the title parts (`VTS_nn_m.VOB`, m ≥ 1) in name order; menu files (`VIDEO_TS.VOB`, `VTS_nn_0.VOB`) SHALL be skipped there but remain playable when chosen on their own. Play Folder on the disc folder SHALL look inside its `VIDEO_TS`. Only unencrypted (already copied) discs can play; `.vob`, `.m2ts` and `.mts` files are video.

DVD-Video disc images (`.iso`) on a local disk SHALL play their main title (the largest title set) as one continuous video, read in place from the image without mounting, extracting or temporary files; the title's parts SHALL play without a gap. Images that are not DVD-Video (installers, data discs, Blu-ray) SHALL be refused with an error naming the reason, and SHALL be skipped when a folder is dropped or played. Remote images SHALL be refused rather than downloaded.

#### Scenario: DVD image
- **WHEN** the user plays `Movie.iso`, a DVD-Video image whose film is split into VTS_01_1.VOB and VTS_01_2.VOB
- **THEN** the film plays from start to end as one item, with seeking over its whole length

#### Scenario: Data image in a folder
- **WHEN** a folder holding `linux.iso` and `song.mp3` is dropped on the Media pane
- **THEN** only `song.mp3` is added

#### Scenario: DVD folder
- **WHEN** a folder `Movie` with `VIDEO_TS/VIDEO_TS.VOB`, `VTS_01_0.VOB`, `VTS_01_1.VOB` and `VTS_01_2.VOB` is dropped on the Media pane
- **THEN** a playlist folder "Movie" holds VTS_01_1 and VTS_01_2 in that order, without the menus

### Requirement: Jump from the playlist to the file
The context menu of a playlist row SHALL offer "Show in folder", which opens the row's folder in a Files pane (the focused one if it is a Files pane, otherwise the first) with the file selected and scrolled into view, and gives that pane focus; playback SHALL continue. The context menu of a playlist folder SHALL offer "Go to folder on disk", which opens the folder it was made from in a Files pane; for a folder made in the playlist the entry SHALL be disabled. Both dispatch `media.reveal`; without a Files pane they SHALL open a new one beside the focused pane, and for a row or folder that does not exist they SHALL fail with a message and change nothing.

#### Scenario: Show a row's file
- **WHEN** the user right-clicks the playlist row of `/music/Album/Song.mp3` and picks Show in folder
- **THEN** a Files pane shows `/music/Album` with `Song.mp3` selected, and the music keeps playing

#### Scenario: Folder made in the playlist
- **WHEN** the user right-clicks a playlist folder created with "New folder from this"
- **THEN** "Go to folder on disk" is disabled

### Requirement: Entry points
Files and Preview panes SHALL offer Play, Play next, Add to queue and Play folder for supported media (context menu and menu bar); dropping supported files or a folder on a Media pane SHALL play them when dropped on the player and add them when dropped on the playlist (see `media-open-and-play-ux`); a Preview video card SHALL offer "Play in Media pane". If no Media pane exists, Play SHALL switch a suitable pane to `Media` (or open one beside the focused pane) rather than fail.

#### Scenario: Play from a Files pane
- **WHEN** a Files pane has an mp3 selected and `media.play` runs for it with no Media pane open
- **THEN** a Media pane appears and plays it

#### Scenario: Drop on the pane
- **WHEN** three audio files are dropped on the playlist of a Media pane
- **THEN** they are appended to the queue in order and selected; the first plays only if nothing was playing and "Start playing when items are added" is on

### Requirement: Transport and tracks
`media.seek` SHALL move to an absolute or relative position clamped to the item's duration; volume SHALL be 0–100 and persist; speed SHALL follow the playback-speed requirement of `media-audio-pipeline`. For video, the pane SHALL list embedded audio and subtitle tracks and let the user choose one or none (subtitles per `media-subtitles`), and SHALL support frame step while paused.

#### Scenario: Seek past the end
- **WHEN** `media.seek` is given a position beyond the duration
- **THEN** it clamps to the end and the next item starts

#### Scenario: Choose an audio track
- **WHEN** a video with two audio tracks plays and the second is chosen
- **THEN** playback continues from the same position with the second track

### Requirement: Familiar player keyboard controls
The focused Media pane SHALL provide YouTube-style keyboard controls through engine commands, in normal and fullscreen modes:

| Key | Action |
|---|---|
| Space or K | Toggle play/pause |
| J / L | Seek backward/forward 10 seconds |
| Left / Right | Seek backward/forward 5 seconds |
| Up / Down | Increase/decrease volume by 5 percentage points |
| M | Toggle mute |
| 0–9 | Seek to 0%–90% of the current item's duration |
| Home / End | Seek to the start/end of the current item |
| F | Toggle Media pane fullscreen |
| Esc | Leave Media fullscreen when active |
| Shift+N / Shift+P | Play next/previous queue item |
| N / P | Aliases for next/previous queue item |
| Comma / Period | Step backward/forward one video frame while paused |
| < / > | Decrease/increase speed by 0.25×, clamped to 0.5×–3× |
| C | Toggle subtitles off/on, retaining the last selected track |

While a menu or dropdown is open, its keys (Esc, arrows, Enter) SHALL go to that menu. A key that does nothing in the current state (Esc outside fullscreen) SHALL be left for other widgets.

These shortcuts SHALL NOT intercept terminal input, text editing, modal dialogs or keys owned by an interactive widget (including a focused slider or queue navigation). Each key event SHALL trigger at most one action. Unsupported actions SHALL be disabled: percentage/end seeking requires a known duration, frame stepping requires paused video, and subtitle toggling requires an available track. Subtitle toggling with no prior selection SHALL select the first available track. Seeking and volume SHALL use the normal clamping and end-of-item rules. A discoverable shortcut reference SHALL be available from the Media pane, and control tooltips SHALL include their shortcuts.

#### Scenario: Familiar seeking
- **WHEN** the Media pane owns keyboard input at 1:00 and the user presses J, then Right
- **THEN** the position moves to 0:50, then 0:55, without changing pause state

#### Scenario: Percentage seeking
- **WHEN** an item with a known duration of ten minutes plays and the user presses 5
- **THEN** playback seeks to five minutes

#### Scenario: Typing is not playback control
- **WHEN** a terminal, text editor, text field or modal dialog owns keyboard input and the user presses K, J, Space or F
- **THEN** playback and fullscreen state remain unchanged and the input owner receives the keys

#### Scenario: Widget owns arrows
- **WHEN** a volume slider has keyboard focus and the user presses Right
- **THEN** the slider handles the key and no playback seek is dispatched

#### Scenario: Frame and speed controls
- **WHEN** a video is paused and the user presses Period, then >
- **THEN** it advances one frame and increases the configured playback speed by 0.25× without resuming

### Requirement: Audio output and A/V sync
Audio SHALL be the master clock. For video, frames SHALL be dropped when late and held when early so that picture and sound stay in sync; a video without audio SHALL use a monotonic clock. Memory SHALL stay bounded: only the newest few decoded frames and a bounded audio buffer are kept however slow the UI is. If no audio device is available, video SHALL play silently with a hint and audio items SHALL report an error and be skipped.

#### Scenario: Slow UI
- **WHEN** the UI is stalled for two seconds during video playback
- **THEN** playback resumes at the clock position by dropping frames, and memory does not grow

#### Scenario: No audio device
- **WHEN** a video plays on a machine with no audio device
- **THEN** it plays silently and the pane shows a hint

### Requirement: Fullscreen
`media.fullscreen` SHALL toggle a mode in which the Media pane fills the screen: all other panes are hidden with their state preserved, and the window itself goes native fullscreen. Leaving SHALL return the window to its previous state, so a window that was already fullscreen (`window.fullscreen`) stays fullscreen. `window.fullscreen` SHALL toggle native window fullscreen on its own and the two SHALL be usable together. In fullscreen the transport controls and cursor SHALL auto-hide after three seconds without pointer movement and reappear on movement. Esc, F or a double-click SHALL leave fullscreen, restoring the previous layout, focus and window state; fullscreen SHALL NOT overwrite the saved normal window geometry. Playback SHALL continue uninterrupted when entering or leaving fullscreen. Closing the pane or a stopped queue SHALL leave fullscreen.

#### Scenario: Enter and leave
- **WHEN** a video plays and the user presses F, then Esc
- **THEN** the video fills the screen, then the previous layout, selection, focus and window size return, with playback never interrupted

#### Scenario: Window already fullscreen
- **WHEN** the window is fullscreen (F11) and the user enters and leaves Media fullscreen
- **THEN** the window is still fullscreen afterwards

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
All media behaviour SHALL be commands with id, label, menu path, shortcut, params documentation and `enabled`: `media.play`, `media.play_next`, `media.pause`, `media.toggle`, `media.stop`, `media.next`, `media.prev`, `media.seek`, `media.volume`, `media.mute`, `media.speed`, `media.enqueue`, `media.play_folder`, `media.remove`, `media.move`, `media.clear`, `media.shuffle`, `media.repeat`, `media.track.audio`, `media.track.subtitle`, `media.step`, `media.fullscreen`, `media.playlist.load`, `media.playlist.save`, `media.loop`, `media.sleep`, `media.eq`, `media.skip`, `media.chapter`, `media.subtitle.load`, `media.subtitle.delay`, `media.bookmark`, `media.finished` and `media.book`. Bad, oversized or malformed params SHALL return an error and never panic; the command fuzzer SHALL cover them.

#### Scenario: Malformed params
- **WHEN** `media.seek` receives a string, a negative index, NaN or an object without the expected fields
- **THEN** it returns an error and playback is unchanged

### Requirement: Playing time in the status bar
While the status bar shows the media controls, it SHALL also show the playing position as `elapsed / total` in the player's clock format (`m:ss`, or `h:mm:ss` from an hour), or only `elapsed` when the length is unknown, in fixed-width digits with theme colours. While playing, the readout SHALL be redrawn when the shown second changes and not more often for its own sake; while paused or minimized it SHALL NOT cause redraws.

#### Scenario: Media pane hidden
- **WHEN** a song plays, the Media pane is closed and the user works in a Files pane
- **THEN** the status bar shows e.g. `1:23 / 4:56` next to the controls and it advances once a second

#### Scenario: Paused
- **WHEN** playback is paused at 2:05 of a 1:02:00 audiobook
- **THEN** the readout shows `2:05 / 1:02:00` and the window draws no frames for it

### Requirement: Video frames reuse one texture
Each video picture (Media pane, Preview pane) SHALL keep one GPU texture exactly the size of its frames: a frame of the same size SHALL overwrite it in place, and a frame of a new size (another video, a resized pane) SHALL re-specify it once. The texture SHALL survive up to 5 seconds of frames without a picture (seek, decoder restart) and be released after that or when its pane closes.

#### Scenario: Resizing while playing
- **WHEN** a 1080p video plays and the pane is resized so the decode width changes several times
- **THEN** the texture is re-specified once per decode-width change, never per frame, and memory does not grow with each resize

#### Scenario: Seeking
- **WHEN** the user seeks repeatedly in a playing video
- **THEN** the same texture is used across the decoder restarts

