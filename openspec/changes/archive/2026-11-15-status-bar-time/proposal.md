## Why

The status bar shows media controls (volume, previous, play/pause, next) once something plays, but not where playback is; with the Media pane hidden or another workspace in front there is no way to see the time without switching (requested 2026-11-14).

## What Changes

- A small time readout next to the status bar's media controls, `elapsed / total` (only `elapsed` when the length is unknown), in the player's clock format, shown whenever those controls are.
- Fixed-width digits so the bar does not shift as the time changes; theme tokens for both themes; a tooltip naming it.
- No extra frames: while playing it asks for a redraw when the shown second changes (not every frame, and not at all while paused or minimized); a playing video on screen already redraws more often.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `media-pane`: new requirement "Playing time in the status bar".

## Impact

`crates/ui/src/media.rs` (`indicator`), shared clock formatting in `player_controls.rs`, a UI test and a snapshot. No engine change.
