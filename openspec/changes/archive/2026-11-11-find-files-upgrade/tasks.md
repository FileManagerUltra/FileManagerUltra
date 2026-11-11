## 1. Shortcuts and typing

- [x] 1.1 `search.open` Ctrl+F (Preview keeps Find in preview), `search.open_everywhere` Ctrl+Shift+F with pseudo-folder skipping; Alt+F7 removed; File menu entry
- [x] 1.2 `view.filter {focus}`; typing in the listing opens the filter (leading space ignored, cursor at end); type-to-go removed; Log Follow shortcut removed
- [x] 1.3 Tests: chord routing (Files, Preview), everywhere root, pseudo folders, type-to-filter end to end

## 2. Background searches

- [x] 2.1 `search.minimize` / `search.restore`; window hidden while minimised, run continues
- [x] 2.2 Jobs panel row (query, status, Show, Stop) and status-bar text for a running search
- [x] 2.3 Tests: minimise keeps running, restore shows results, stop from the panel, close stops

## 3. History

- [x] 3.1 `Prefs::search_history` (20, newest first, deduplicated, capped); recorded on start
- [x] 3.2 Recent searches in the window; `search.history.run {index}`, `search.history.clear`
- [x] 3.3 Tests: cap, dedupe, persistence round trip, hostile values, bad params

## 4. Speed

- [x] 4.1 Local walk without per-entry `stat`; matches stat'ed
- [x] 4.2 Parallel walk (≤ 8 threads) with caps, cancel and symlink rules unchanged
- [x] 4.3 `memchr::memmem` content matching; batched result publishing
- [x] 4.4 Benchmark before/after on a large tree; tests: same results as before, cancel, caps

## 5. Close

- [x] 5.1 Docs (architecture walker notes, development smoke steps), CHANGELOG, devlog; gate
