## 1. Layout state

- [x] 1.1 Add `on_current: bool` (`#[serde(default)]`) to `ListingState`, with `focus_current()`, clearing in `select_only`/`toggle`/`extend_to`/`select_all`/`clear_selection`, and the "at most one special row" invariant documented.
- [x] 1.2 Extend `move_focus_with_parent` (rename to `move_focus_with_specials`) so Up/Down walk `.` ⇄ `..` ⇄ first entry, Home lands on `.`, and Shift never extends onto a special row.
- [x] 1.3 Make `navigate`/`go_back`/`go_forward` reset the cursor to `.` via one helper; keep the same-location navigate no-op untouched.
- [x] 1.4 Tests: special-row walks, Home, selection isolation, navigate/go_back/go_forward default, same-location navigate keeps the place, serde default for old sessions.

## 2. Engine

- [x] 2.1 `cursor_entry` resolves `on_current` to a synthetic `Entry` for the pane's location (directory kind, name `.`).
- [x] 2.2 Test: with the cursor on `.`, the followed Preview slot shows the current directory (Dir body / subject URI equals the pane location); `..` still yields no preview.

## 3. UI

- [x] 3.1 Render the `.` row above `..`: row count, click-to-focus, focused/hover painting, "Current folder" hover text, no drag, activation ignored.
- [x] 3.2 Update keyboard/scroll math (cursor row, page jumps, Home/`Absolute(0)`, `PageUp` clamp, scroll-into-view) for the extra row; focus check in `entry_row_ui` excludes special rows.
- [x] 3.3 Replace the empty-directory `on_parent` hack with "cursor on `.` when no entry row is owned", without overriding an explicit `.`/`..` choice.
- [x] 3.4 Tests in `pane.rs`: Up from first entry reaches `..`, second Up reaches `.`, Enter/double-click on `.` does not navigate, Home lands on `.`, empty directory starts on `.`.
- [x] 3.5 Regenerate the listing snapshots (`UPDATE_SNAPSHOTS=1 cargo test -p fm-ui --test snapshots`), confirm both palettes show `.` first, and remove `*.new.png`.

## 4. Specs, docs and gate

- [x] 4.1 Verify the `file-listing` delta covers the requirement text it replaces (`..`-first) and matches the implemented behaviour.
- [x] 4.2 Update `docs/architecture.md` / `docs/roadmap.md` where the listing's leading rows are described; add a `docs/development.md` smoke line (tree select → `.` focused → Preview shows the folder).
- [x] 4.3 `cargo fmt --all && cargo xtask ci` green; `openspec validate --all` green; `log/devlog.md` entry with test counts.
