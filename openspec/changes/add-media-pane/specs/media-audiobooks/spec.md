## ADDED Requirements

### Requirement: Audiobooks
An item SHALL be treated as an audiobook when it is an `.m4b`, its genre tag is "Audiobook", or the user marks it or its folder as an audiobook. A folder of audio files marked as one book SHALL play as a single timeline in natural name order, with total duration, total position and one chapter per file when the files carry none. Embedded chapters (m4b/MP4, MP3 CHAP, Matroska) SHALL be listed, selectable, and navigable with next/previous chapter. Audiobook controls SHALL add skip back 15 s and forward 30 s buttons (configurable to 15/30 s), speed 0.5×–3× remembered per book, and progress as a percentage.

#### Scenario: Multi-file book
- **WHEN** a folder of 12 mp3 files is played as a book
- **THEN** the pane shows one timeline and progress across all files and seeking crosses file boundaries

#### Scenario: Chapters
- **WHEN** an m4b with chapters plays and "next chapter" runs
- **THEN** playback jumps to the start of the next chapter

#### Scenario: Skip buttons
- **WHEN** "skip back 15 s" runs at 0:10
- **THEN** playback goes to 0:00 and does not go before the start

### Requirement: Resume position
The position of every audiobook and every video SHALL be remembered (by path or `VfsUri`, with file size and modified time to detect replaced files) and playback SHALL resume there when the item is played again, after a short rewind of 3 s. Music tracks SHALL NOT be remembered. The store SHALL keep the 500 most recently played items, hold paths and positions only, be owner-only, and be clearable from Preferences. An item played to within 30 s of its end (or 98 %) SHALL be treated as finished and restart from the beginning.

#### Scenario: Resume a video
- **WHEN** a video was closed at 42:10 and is played again
- **THEN** it resumes at about 42:07

#### Scenario: Replaced file
- **WHEN** the file's size or modified time changed since the position was saved
- **THEN** playback starts from the beginning

#### Scenario: Cap
- **WHEN** a 501st item is remembered
- **THEN** the least recently played entry is dropped

### Requirement: Bookmarks and finished state
`media.bookmark` SHALL add, remove and jump to named bookmarks (position and optional note) for an audiobook or video, persisted with the resume store and capped at 100 per item. An audiobook SHALL show progress as a percentage and be markable as finished or unfinished with `media.finished`, which updates what the queue and folder views show.

#### Scenario: Add and return to a bookmark
- **WHEN** a bookmark is added at 1:05:00 and the user later jumps to it
- **THEN** playback continues from 1:05:00

#### Scenario: Mark finished
- **WHEN** an audiobook is marked finished
- **THEN** its progress shows 100 % and it restarts from the beginning next time
