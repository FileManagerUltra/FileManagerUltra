## Context

- Activation: `open_cmds::plan` turns a double-click or Enter into `nav.open`, `file.open {mode: "default", confirm: true}` or a program confirmation. Media files always go to `file.open`, so the Media pane is only reached through the context menu or the Media menu.
- The Jobs panel (`fm-ui::jobs`) already shows jobs, a bounded `MessageLog` (`engine::messages`), the update notice with buttons (`fm-ui::updates`) and minimised searches. Plain messages are text-only (`Message { text, at }`).
- The Media pane (`fm-ui::media`): a playlist with Files-style selection (`fm_layout::selection::RangeSelection`, `MediaPaneState::picked`/`focused_row`), row double-click already runs `media.play {index}`, folder rows accept drops, and the video area maps click to `media.toggle` and double-click to `media.fullscreen`. External OS drops reach `fm-app` (`main.rs`) and are sent as `media.drop {uris}` with no position. `media.drop` auto-plays when idle.
- The `media-pane` spec lives in the unarchived `add-media-pane` change; this change adds requirements next to it.

## Goals / Non-Goals

**Goals:**
- Make in-app playback discoverable with one unobtrusive hint, never a modal.
- Make the result of every add and drop visible and predictable.
- Put player settings one right-click away, built only from existing commands.

**Non-Goals:**
- Changing the OS default application or registering the app as a media handler.
- An "Ask every time" option (it would turn every double-click into a decision, which is the invasiveness we want to avoid).
- New player features: the menu only gathers what already exists.
- Remote playback (still copy-first).

## Decisions

1. **Routing lives in the activation plan, not in `file.open`.** `plan()` already classifies entries. A media file whose kind is set to *Play in Media pane* yields `media.play {uris: [uri]}`, everything else is unchanged. `file.open` stays a pure "OS handler" command, so its security requirements (local-only, shell-free, program confirmation) are untouched. *Alternative:* a `mode: "media"` on `file.open`, rejected because it mixes two very different side effects in one command.
2. **Media kind uses the existing classification.** It uses the extension and sniffing tables `media_entry_cmds`/`fm-media` already use to decide what Play accepts, so "playable" means the same everywhere. Playlists and `.iso` keep `file.open` on activation (loading a playlist or a disc image on a double-click would be surprising).
3. **The hint is a message with actions, not a new widget.** `Message` gets `actions: Vec<MessageAction { label, command, params }>` (≤ 3, labels ≤ 40 chars), a `kind` (`Plain`/`Hint`) and a `hint_id`. The panel renders buttons and dispatches by id, the same way the update notice does, so the UI stays a thin dispatcher. Hint messages use a 15 s auto-close, plain ones keep 5 s. The hint is pushed by `file.open` (mode `default`) right after it hands a local file to the OS, only when the file is media the pane plays, its kind is set to the default app, and the id is not in `Prefs::hints_seen`. The id is inserted at the same moment. Prefs are autosaved shortly after any change and on exit, so a restart does not show it again. `hints_seen` is a bounded set (≤ 64 ids, ≤ 64 bytes each, unknown ids dropped on load).
4. **Actions:** *Play in Media pane* → `media.play {uris}`; *Always play here* → `prefs.media_open {kind, target: "media"}` then `media.play {uris}` (two dispatches from one click, run in order); *Don't show again* → `hint.dismiss {id}`. Any click also marks the message's actions as spent.
5. **Rename to "Activity".** It covers what the panel holds (jobs, messages, hints, updates, minimised searches) in one word and matches common desktop wording. Only user-visible strings, accessibility labels, specs and docs change. Code identifiers (`jobs.rs`, `jobs_open`) stay to keep the diff small; a later refactor can rename them. Alternatives considered: "Notifications" (misses jobs), "Tasks" (sounds like a to-do list), "Messages" (misses progress).
6. **Playlist reveal via a one-shot engine signal.** `MediaState` gets `reveal: Option<RevealRange { start, len, seq }>`, set by every queue insert path (`submit_media_sources`, enqueue, play_next, place, playlist load) after skips are known. The pane keeps the last `seq` it consumed in `MediaPaneState` (not persisted). On a new `seq` it selects the range, opens a collapsed folder that holds it, and centres the first row only when it is not fully visible. The scroll target is kept for three frames and clamped to the previous frame's maximum, because egui clamps to last frame's content and rubber-bands past it. Focus moves only for drops on the Media pane (`dnd::finish_media` focuses the pane under the pointer, and OS drops already do). Moving focus after a Play from a Files pane would hand the arrow keys to volume and seek. A seq number (instead of clearing the field) lets a hidden pane consume it later without races.
7. **Drop targets are decided in the UI, by rect.** The Media pane records its player rect and playlist rect each frame in egui temp data (or in a small `MediaDropZones` published to `fm-app`). In-app drags use egui's drag payload hover on each zone. OS drops in `fm-app` look up the pointer position at drop time: player rect → `media.drop {uris, play: true}`, playlist rect → `media.drop {uris, after/before/group}` from the hovered row, anywhere else in the pane → playlist end. Zone highlight uses theme tokens (`drop_target` / `drop_target_border`). `media.drop` drops its "auto-play when idle" rule. `play: true` routes to the same insert-after-current-and-start path as `media.play {uris}`.
8. **Context menu reuses the toolbar's menus.** `player_menu` calls the same `track_menus`/`subtitle_menu` and `media_audio::menus` the Options button uses, so there's one source of truth. A click-sensing background is laid out first over last frame's player rect, so every control drawn on top keeps its own clicks. The video picture's own response is checked too. Labels in the player area are not selectable, or they would take the right-click. Player keys come from `media_keys::HELP` (the media commands have no registry shortcuts). Right-click never records the pending single click (that timer only starts on a primary click). Keyboard: Shift+F10 while the Media pane has focus opens the same menu at the player centre. "Video fit" was dropped: `media.video.fit` is an internal width report, not a user setting.

## Risks / Trade-offs

- [Users who never look at the Activity panel miss the hint] → It opens the panel itself, stays 15 s, and the preference is also listed in Preferences → Media.
- [Two drop zones confuse users who expect one] → Both zones highlight on hover with different labels ("Play now" over the player, "Add to playlist" over the list), so the outcome is visible before release.
- [Losing auto-play on an idle playlist drop surprises existing users] → Dropping on the player covers that case. The change is called out in the changelog.
- [Focus steal while the user types elsewhere] → Reveal never takes focus from a text field, terminal or dialog. It only selects and scrolls.
- [Pointer position for OS drops differs per platform (X11 XDND, Wayland, Windows OLE)] → Fall back to the playlist (the non-playing, safer choice) when the position is unknown.
- [Preference migration] → New fields are `#[serde(default)]`, so existing prefs files load unchanged with *Open in default app* and an empty `hints_seen`.

## Open Questions

- Should the hint also appear the first time a media file is opened via Open With…? (Proposed: no, because the user picked an app on purpose.)
