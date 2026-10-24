## Context

`ListingState` (`crates/layout/src/state.rs`) already models one synthetic leading row: `on_parent: bool` means "the cursor is on `..`, no entry is focused", and `focused_row` always indexes the entry order. `crates/ui/src/listing.rs` offsets display rows by that boolean in five places (row count, keyboard cursor mapping, scroll target, painting, focused check), and `crates/engine/src/preview_cmds.rs::cursor_entry` returns `None` for `..` so no preview follows it.

Directories can currently only be previewed by moving the cursor off them, which is why the user asked for a `.` row that can be focused (and for new directories to start there). The preview layer already renders `Preview::Dir` (summary + thumbnail grid), so the change is about making the cursor addressable, not about new rendering.

## Goals / Non-Goals

**Goals:**
- A focusable `.` row that resolves to the pane's own directory for the Preview pane.
- Special rows (`.`, `..`) never enter the name-keyed selection and are never drag sources; `..` keeps working exactly as today.
- Navigating to a folder (tree, `nav.open`, history) leaves the cursor on `.`; refresh keeps whatever the cursor was on.
- No new state on disk that breaks old session files.

**Non-Goals:**
- Selecting `.` as part of a multi-selection for copy/move/delete (copying a folder into itself needs conflict handling that no backend supports).
- Making `.` a drop target or drag source (the directory itself is still reachable through `..` in its parent).
- Changing what `nav.up` does when the cursor is on `.`.

## Decisions

**D1 — Two booleans instead of a `CursorRow` enum.** `ListingState` gains `on_current: bool` beside `on_parent`, both cleared by `select_only`/`toggle`/`extend_to`/`select_all`/`clear_selection`, both `#[serde(default)]`. A `enum Cursor { Entry(usize), Current, Parent }` would be cleaner but rewrites every use of `focused_row` (layout, UI, engine, session persistence, tests) for a two-row feature. The booleans keep `focused_row` meaning "entry index" and make the diff reviewable. Invariant documented in the struct: at most one of `on_current`/`on_parent` is true.

**D2 — `.` is always present, `..` only below the root.** The current folder exists at every location including the filesystem root, where `..` does not; drawing both unconditionally would need a fake parent. Root listings therefore show `.` alone plus entries, which also removes the old "(empty)" dead-end at the root.

**D3 — Navigation resets the cursor to `.`; refresh does not.** `PaneState::navigate` already resets `focused_row`/selection/scroll; it (and `go_back`/`go_forward`) will call one `reset_cursor()` helper that sets `on_current = true`. The refresh path never touches `ListingState`, so `nav.refresh` keeps the cursor on the same entry — this preserves the `multi-pane-ui` guarantee that a reload does not move the pane. Tree selection goes through `navigate`, so it inherits the same rule.

**D4 — Engine synthesizes the entry.** `cursor_entry` returns `Entry::new(location, EntryKind::Dir)` with `name = "."` when `on_current` is set. The preview layer keys jobs by `(uri, kind)`, so this produces the same `Preview::Dir` the folder would get from any other path, with no special case in the preview crate. The alternative (teaching `PreviewSlot` about the pane's location) duplicates the cursor contract in a second place.

**D5 — `.` activation is inert, not a no-op navigate.** `navigate` to the same URI is now a no-op, so dispatching it would be harmless, but double-clicking `.` should not look like a reload; the UI therefore ignores activation on that row. `..` keeps its navigate-on-click behaviour.

**D6 — Empty-directory auto-focus points at `.`.** The existing hack that forces `on_parent` when a directory has no entries is replaced by "if no row owns the cursor, focus `.`". It no longer fights the user: pressing Up from `.` to `..` in an empty directory stays on `..` across frames.

## Risks / Trade-offs

- [Every command that reads `on_parent` must also read `on_current`] → Only `cursor_entry`, the row math and the focused check do; each gets an explicit branch and a test. `rg on_parent` is small enough to audit.
- [Existing snapshots and pane unit tests assume `..` in row 0] → Regenerate the listing PNGs (`UPDATE_SNAPSHOTS=1`) and move the two `pane.rs` cursor tests to the new rows.
- [Users who expect the first child focused after entering a folder] → This is the requested default; the selected row is one Down press away and the rule is uniform across tree, `nav.open` and history.
- [`cursor_entry` now clones a synthetic entry each poll] → One `VfsUri` clone per frame for the pane that has a followed Preview; negligible against the existing per-frame view borrow.
- [Old session files have `on_current` missing] → `#[serde(default)]` keeps them loadable with the cursor on `focused_row` as before.
