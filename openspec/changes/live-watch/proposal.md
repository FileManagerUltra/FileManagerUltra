# Live directory watching

Local panes refresh automatically when their directory changes on disk, on Linux, macOS and Windows, using the `notify` crate (inotify / FSEvents / `ReadDirectoryChangesW`). Remote panes (SFTP, SMB) and mounts where native events are unreliable (Windows mapped/UNC drives, network mounts) fall back to a visible-only poll plus refresh on focus regain.

## Why

Panes only refresh on `nav.refresh` (Ctrl+R) or after the app's own operations. Files created, renamed or deleted by other programs (terminals, downloads, builds) leave stale listings. `docs/architecture.md` already reserves the design: `DirCache::invalidate` is the only mutation path and the watcher only invalidates.

## Scope

- Engine watcher: non-recursive watch per directory displayed by any pane, reference counted across panes/tabs, 100 ms debounce, events only call `DirCache::invalidate` (never `read_dir` inline). Overflow/rescan events invalidate the whole directory. A vanished watched directory reports an error state, not a crash.
- Watch lifecycle follows the set of visible local URIs (navigation, split/close, tab switch); watchers for hidden tabs are dropped and re-created on activation.
- Polling fallback for remote schemes (5 s, focus regain) and local folders without a live watch (2 s: network filesystems, UNC, watch cap/failure); never on the UI thread, on the matching job lane.
- Selection, scroll position and focus survive a refresh when the entries still exist (by name).
- Own operations (copy/move/delete) must not produce double refreshes.
- Limits: cap on concurrently watched directories (inotify watch exhaustion is surfaced as a notice and falls back to polling).

## Out of scope

- Recursive watching, file-content watching for previews (existing reload commands remain).
- Session/pane persistence (already delivered by `workspaces`) and installer packaging (already in `release.yml`); remaining packaging smoke checks stay tracked in the roadmap and are not part of this change.

## Capabilities

- Modified: `directory-cache` (watcher-driven invalidation, polling fallback), `security` (watcher limits, hostile event floods).
