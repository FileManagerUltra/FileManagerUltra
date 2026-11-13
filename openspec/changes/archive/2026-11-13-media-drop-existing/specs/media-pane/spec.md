## MODIFIED Requirements

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
