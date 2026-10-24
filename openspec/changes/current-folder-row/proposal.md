## Why

The listing can only put the cursor on entries and the `..` row, so the folder you are actually looking at is the one thing a pane cannot target. Selecting the folder itself matters for the Preview pane: a folder preview (summary or thumbnail grid) is only reachable by entering the folder, at which point the cursor has already moved to a child.

## What Changes

- Show a `.` (current folder) row as the first row of every Files listing, with `..` below it when the pane is not at its root.
- The `.` row is focusable, never part of the selection, is not draggable, and does not navigate when activated — it exists so the pane's own folder can be previewed (and later targeted by commands).
- The engine resolves the cursor on `.` to a synthetic directory entry for the pane's location, so the Preview pane opens the folder summary / thumbnail grid.
- **BREAKING (behavioural)**: opening a directory (tree selection, `nav.open`, back/forward) now starts with the cursor on `.` instead of the first child; empty directories likewise start on `.`.
- Update the `file-listing` spec (`..`-first requirement replaced) and refresh the affected egui snapshots.

## Capabilities

### New Capabilities
- (none)

### Modified Capabilities
- `file-listing`: the leading-rows requirement changes from "`..` is always first" to "`.` then `..`", and the default cursor after navigation moves to `.`.

## Impact

- `crates/layout`: `ListingState` gains `on_current` (serde-defaulted), new focus helpers, and `navigate`/`go_back`/`go_forward` reset the cursor to `.`.
- `crates/engine`: `cursor_entry` resolves `.` to the pane's own directory entry (preview follows it); one engine test covers the folder preview.
- `crates/ui`: listing renders the `.` row, keyboard/scroll math accounts for it; snapshot fixtures gain the row.
- Spec/tests: `file-listing` delta, layout/UI/engine tests, regenerated PNGs. No new dependencies.
