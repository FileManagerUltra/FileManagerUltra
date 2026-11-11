# Find Files upgrade

## Why

Find Files sits behind Alt+F7, searches one folder at a time, blocks its window while it
runs, forgets every search, and walks slowly: each listed entry costs two `stat` calls on
a single thread. Users expect Ctrl+F to find files, a way to search the whole computer,
background searches they can watch like a copy, and quick re-use of earlier searches.

## What Changes

- Ctrl+F opens Find Files for the focused folder (Preview keeps Ctrl+F for Find in
  preview); Ctrl+Shift+F opens it for the whole computer (`search.open_everywhere`: `/`,
  or every drive on Windows), skipping system pseudo-folders. Alt+F7 is removed.
- Typing in a Files listing opens the quick filter with the typed text. Type-to-go (typing
  opened the location bar) is removed; Ctrl+L and clicking the path still edit it.
  Log Follow loses its shortcut (menu and Follow button remain).
- A running search can be minimised to the jobs panel, with live status there and in the
  status bar, and reopened or stopped from it (`search.minimize`, `search.restore`).
- The last 20 searches (folder, name, text, hidden) are remembered across restarts and can
  be re-used from the Find Files window (`search.history.run`, `search.history.clear`).
- Faster walk: local searches read entry types without `stat` (only matches are
  stat'ed), walk folders on several threads, use SIMD substring search for content and
  publish results in batches. Caps and symlink rules are unchanged.

## Impact

- `fm-engine`: `search_cmds` (commands, walker, history), `prefs` (history field),
  `select_cmds` (`view.filter {focus}`), `preview_cmds` (no Log Follow shortcut).
- `fm-ui`: `listing` (type-to-filter), `locationbar` (type-to-go removed), `search`,
  `jobs`; `fm-app`: shortcut table and File menu.
- Specs: `file-search`, `file-listing`.
