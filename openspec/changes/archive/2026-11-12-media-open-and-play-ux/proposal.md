## Why

The Media pane is easy to miss: double-clicking an audio or video file always opens the OS default app, and users never learn they can play it inside the file manager. Once they do use the pane, a few things feel slow or hidden: added items can land off-screen in the playlist, a drop never says whether it should play, and the player's settings (equalizer, subtitles, tracks, speed) live only in the toolbar.

## What Changes

- New preference (Preferences → Media), separate for audio and video: **When I open an audio file / a video file**: *Open in default app* (default, today's behaviour) or *Play in Media pane*. With *Play in Media pane*, activating a media file (double-click, Enter) runs Play: the file is inserted after the current item and starts at once.
- One-time hint: the first time a user opens a media file in the default app, the panel that shows jobs and messages pops up with *"You can also play audio and video here."* plus **Play in Media pane**, **Always play here** and **Don't show again** buttons. The file still opens in the default app first. The hint never comes back after any choice, after it is dismissed, or after it has been shown once.
- The **Jobs panel is renamed "Activity"** (title, status-bar icon tooltip and accessibility label, docs). It already holds jobs, messages, update notices and minimised searches, so "Jobs" no longer fits. The panel's messages can now carry action buttons, as the update notice already does.
- Playlist follows additions: after any add (Play, Play next, Add to queue, Play folder, drop, load playlist) the added rows are selected, the first one is scrolled into view and the playlist gets keyboard focus.
- Drops split by target: dropping on the **player** (video picture, cover or transport area) plays the files at once (Play semantics). Dropping on the **playlist** only adds them there, at the drop position, and selects them; it no longer auto-starts an idle player. **BREAKING** (behaviour): a drop on the playlist with nothing playing used to start playback.
- Right-clicking the player (video picture or audio cover area) opens a context menu with the player's settings and actions: play/pause, stop, previous/next, audio track, subtitles (track, load file, delay, appearance), equalizer, speed, loop and sleep timer, video fit, fullscreen and Show in folder. Every entry dispatches an existing command.

## Capabilities

### New Capabilities
- `media-open-preference`: the per-kind "open media files in" preference, routing of file activation to `media.play`, and the one-time discovery hint with its actions.

### Modified Capabilities
- `job-progress`: the Jobs panel is renamed Activity, and panel messages can carry action buttons (used by the media hint).
- `media-pane` (in-flight in `add-media-pane`, not yet archived): the playlist follows newly added items, drops behave differently on the player and on the playlist, and the player gets a context menu. The `Entry points` requirement's "Drop on the pane" scenario in `add-media-pane` is updated in this change so the two changes do not contradict each other.

## Impact

- `fm-engine`: `Prefs` gets `media_open_audio` and `media_open_video` (enum, default `DefaultApp`) plus a `hints_seen` set; the activation plan routes media files; `media.drop` gets a `play` flag; new commands `prefs.media_open` and `hint.dismiss`; `MessageLog` entries can carry actions; a "rows to reveal" signal for the playlist after additions.
- `fm-ui`: the Activity panel (rename, message action buttons), Media pane drop targets, playlist scroll-to and focus, player context menu, Preferences → Media section.
- `fm-app`: external file drops go to the player or the playlist depending on where the pointer is.
- Specs and docs: `job-progress`, `file-search` and `docs/` wording ("jobs panel" → "Activity panel"); `add-media-pane` drop scenario.
- No new dependencies; no new I/O paths (activation still goes through `file.open` or `media.play`).
