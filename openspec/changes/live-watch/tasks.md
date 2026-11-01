## Implementation

- [x] Add `notify` to `fm-engine`; `engine::watch` with refcounted non-recursive watches, event channel and path normalization (Windows drive/`\\?\`/UNC handling).
- [x] Debounce/coalesce in `Session::poll`; invalidate only; merge with op-driven invalidations.
- [x] Reconcile watched set on navigation, split/close and workspace tab switch.
- [x] Polling fallback (remote 5 s + focus regain; local non-event folders 2 s) and watch-creation failures; unchanged-listing detection.
- [x] Preserve selection/focus by name and clamp scroll on refresh.
- [x] Watch/event caps and notices (inotify limit, overflow, vanished directory).
- [x] Tests: tempdir create/rename/delete, burst, overflow, vanished dir, refcount, poll no-op, hostile paths, selection preservation.
- [x] Docs: architecture (Watching), roadmap, specs sync, devlog.
- [ ] Manual smoke checks on Windows and macOS (including a mapped network drive).
