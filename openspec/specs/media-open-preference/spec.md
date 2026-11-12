# media-open-preference Specification

## Purpose
TBD - created by archiving change media-open-and-play-ux. Update Purpose after archive.
## Requirements
### Requirement: Opening media files follows a per-kind preference
The app SHALL keep two preferences, one for audio and one for video files, each either *Open in default app* (the default) or *Play in Media pane*, editable in Preferences → Media and with `prefs.media_open {kind: "audio" | "video" | "both", target: "default" | "media"}`. Activating a supported media file (double-click or Enter in a Files pane or the tree) SHALL dispatch `file.open` as before when its kind is set to *Open in default app*, and `media.play {uris: [uri]}` when it is set to *Play in Media pane*. In the second case the file SHALL be inserted after the current item and start at once, with a Media pane opened as `media.play` already does. Remote files, playlists (`.m3u`, `.m3u8`, `.pls`), disc images and anything the Media pane cannot play SHALL keep the `file.open` path. Context-menu Play, Play next and Add to queue SHALL stay available whatever the preference.

Whichever way a file is opened (`file.open` in the default app, or `media.play` with that file), a Preview pane showing it SHALL pause its video or audio and give up the audio output, so two players never run at once.

#### Scenario: Preview stops when the file is opened
- **WHEN** a video autoplays in a Preview pane after one click, and the user double-clicks it
- **THEN** the Preview pauses while the default app (or the Media pane) plays it

#### Scenario: Default is unchanged
- **WHEN** a new user double-clicks `song.mp3`
- **THEN** `file.open` hands it to the system default application, exactly as before this change

#### Scenario: Play in the Media pane
- **WHEN** audio is set to *Play in Media pane*, `B.mp3` is playing in queue A, B, C and the user double-clicks `song.mp3`
- **THEN** the queue is A, B, song, C and `song.mp3` starts immediately

#### Scenario: Audio and video are separate
- **WHEN** audio is set to *Play in Media pane*, video to *Open in default app*, and the user double-clicks `film.mkv`
- **THEN** `film.mkv` opens in the default application and the Media pane is not touched

#### Scenario: Remote file
- **WHEN** audio is set to *Play in Media pane* and the user double-clicks an mp3 on an SFTP pane
- **THEN** the existing `file.open` behaviour applies, including its copy-first message

#### Scenario: Bad params
- **WHEN** `prefs.media_open` receives a missing or unknown `kind` or `target`, or a non-object
- **THEN** it returns `Err(InvalidParams)` and the preferences are unchanged

### Requirement: One-time hint about playing media in the app
The first time a local media file is opened in the default application with `file.open` while its kind is set to *Open in default app* (Open With… does not count, since the user picked an app), the app SHALL, after handing the file to that application, add a hint message to the Activity panel and open the panel. The hint SHALL read "You can also play audio and video right here in <app name>." (the name from `fm-brand`) and SHALL offer **Play here now** (plays that file in the Media pane now, with no preference change), **Always play here** (sets the preference for that file's kind to *Play in Media pane*, then plays the file) and **Don't show again**. The hint SHALL be shown at most once per user: the moment it is shown, its id SHALL be recorded in `Prefs::hints_seen`, which is saved with the other preferences, so it does not come back after a restart, a dismissal, or when it is ignored. The hint SHALL NOT take keyboard focus, SHALL NOT block the default application, and SHALL stay open until the pointer has left the panel and 15 seconds have passed, longer than the 5 seconds a plain message stays. `hint.dismiss {id}` SHALL close that hint and mark it seen. An id that is not short lowercase ASCII (letters, digits, `-`, `.`), or params that are not `{id: string}`, SHALL return `Err(InvalidParams)`.

#### Scenario: First media open
- **WHEN** a user who has never seen the hint double-clicks `song.mp3`
- **THEN** the default application opens it, and the Activity panel opens with the hint and its three buttons

#### Scenario: Shown once
- **WHEN** the hint has been shown once, whatever the user did with it, and another media file is opened after a restart
- **THEN** the file opens in the default application and no hint appears

#### Scenario: Always play here
- **WHEN** the user chooses **Always play here** on the hint for `song.mp3`
- **THEN** audio is set to *Play in Media pane*, `song.mp3` plays in the Media pane, and the hint closes

#### Scenario: Hint ignored
- **WHEN** the hint appears and the user keeps working without touching the panel
- **THEN** the panel closes by itself after 15 seconds and the hint stays in the message log until cleared

