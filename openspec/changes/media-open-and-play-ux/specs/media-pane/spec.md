## ADDED Requirements

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
The Media pane SHALL have two drop targets, each highlighted while a drag hovers it. Dropping supported files or folders (from a pane, the OS or the playlist itself) on the **player area** (video picture, audio cover or transport bar) SHALL use Play semantics: insert after the current item and start the first dropped item at once. Dropping on the **playlist** SHALL add them at the drop position (beside the hovered row, inside a hovered folder row as now, or at the end below the last row) without changing playback. Nothing starts when nothing is playing, unless the user turned on "Start playing when items are added". The new rows SHALL then be revealed as the playlist-follow requirement says. Drops from other apps SHALL pick the target from the pointer position; when the position is unknown they go to the end of the playlist. `media.drop` SHALL take `play: bool` (default `false`), and `true` SHALL be accepted only without `group`, `after`, `before` or `outside`. Reordering by dragging rows inside the playlist is unchanged.

#### Scenario: Drop on the video
- **WHEN** `B.mkv` is playing in queue A, B, C and the user drops `D.mkv` on the video picture
- **THEN** the queue is A, B, D, C and D starts immediately

#### Scenario: Drop on the playlist while idle
- **WHEN** nothing is playing and the user drops three audio files below the last playlist row
- **THEN** they are appended in order, selected and scrolled into view, and nothing starts playing (with "Start playing when items are added" off, the default)

#### Scenario: Drop between rows
- **WHEN** the user drops `X.mp3` on the playlist between rows 4 and 5
- **THEN** X becomes row 5, is selected, and the current item keeps playing

#### Scenario: Conflicting drop params
- **WHEN** `media.drop` receives `play: true` together with `group` or `after`, or a non-boolean `play`
- **THEN** it returns `Err(InvalidParams)` and the queue is unchanged

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
