## 1. Preferences and commands

- [x] 1.1 Add `MediaOpenTarget { DefaultApp, MediaPane }`, `Prefs::media_open_audio`, `Prefs::media_open_video` and bounded `Prefs::hints_seen` (serde defaults; hostile/oversized entries dropped on load) with round-trip and old-file-loads tests
- [x] 1.2 New module `engine/src/media_open_cmds.rs`: `prefs.media_open {kind, target}` and `hint.dismiss {id}` specs, registered in `commands.rs`; tests incl. bad params → `Err` (fuzz suite passes)
- [x] 1.3 Route activation in `open_cmds::plan`: supported local audio/video → `media.play {uris}` when its kind is set to Media pane; playlists, `.iso`, remote and unsupported files keep `file.open`; tests for each case

## 2. Activity panel and hint

- [x] 2.1 `Message` gets `kind` (`Plain`/`Hint`), `hint_id` and up to 3 `MessageAction { label, command, params }`; actions spent after one click; bounds tests
- [x] 2.2 Push the media hint after a default-app open when unseen; record in `hints_seen` and save prefs at the same moment; tests: shown once, survives restart, not shown with Media pane target
- [x] 2.3 UI: render message action buttons, dispatch by id ("Always play here" = `prefs.media_open` then `media.play`), 15 s auto-close for hints, no focus steal; snapshot in dark and light
- [x] 2.4 Rename user-visible "Jobs" → "Activity" (panel title, status-bar tooltip "Activity: jobs and recent messages", accessibility label, update CTA text if it says Jobs); update affected snapshots
- [x] 2.5 Update "jobs panel" wording in `openspec/specs/file-search/spec.md`, `docs/architecture.md`, `docs/development.md`, `docs/install.md`

## 3. Playlist follows additions

- [x] 3.1 Engine: `MediaState::reveal { start, len, seq }` set by every insert path after skips (Play, Play next, enqueue, play folder, drop, place, playlist load, activation); tests per path incl. skipped items
- [x] 3.2 UI: consume new `seq` once per pane, select the range, scroll the first row into view only when not fully visible, focus the playlist unless text/terminal/dialog has focus; reveal deferred while the pane is hidden; kittest for a 300-row queue

## 4. Drop targets

- [x] 4.1 `media.drop` gets `play: bool` (Play semantics: insert after current and start); remove auto-play-when-idle; reject `play` with `group`/`after`/`before`/`outside` or non-bool; tests incl. fuzz
- [x] 4.2 UI: player and playlist drop zones with hover highlight and labels ("Play now" / "Add to playlist") from theme tokens; in-app drags dispatch per zone
- [ ] 4.2a Pixel snapshot of the two drop zones while dragging (both palettes)
- [x] 4.3 fm-app: OS file drops pick the zone from the pointer position (player → `play: true`, playlist row → `after`/`before`/`group`, unknown position → playlist end)
- [x] 4.4 Update the "Drop on the pane" scenario of `Entry points` in `openspec/changes/add-media-pane/specs/media-pane/spec.md` to match

## 5. Player context menu

- [x] 5.1 `player_context_menu`: transport, Audio track, Subtitles (tracks, Load…, Delay, Appearance… → Preferences → Media), Equalizer…, Speed, Loop, Sleep timer, Video fit and Fullscreen (video only), Show in folder; existing command ids only, shortcuts from registry, disabled/hidden rules, current choice checked
- [x] 5.2 Wire right-click on video picture, cover area and transport bar without triggering the single-click toggle; Shift+F10 / Menu key while the player has focus
- [x] 5.3 kittest: audio vs video entries, empty queue, picking a subtitle dispatches `media.track.subtitle` without pausing

## 6. Preferences UI and docs

- [x] 6.1 Preferences → Media: "When I open an audio file" / "When I open a video file" choices; snapshot
- [x] 6.2 CHANGELOG `[Unreleased]`: Added (play media files in the app on double-click, player right-click menu, playlist follows added items, drop on player plays), Changed (Jobs panel is now Activity; dropping on the playlist no longer starts playback)

## 7. Gate

- [x] 7.1 `cargo test` for engine/ui/app, `cargo clippy --all-targets -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --check`, `openspec validate media-open-and-play-ux --strict`
- [ ] 7.2 Try it in the real app (double-click with both settings, hint once, drops on both zones, right-click menu on audio and video); append a `log/devlog.md` entry
