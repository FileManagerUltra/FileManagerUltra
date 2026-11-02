## Design

- `engine::watch` owns a `notify::RecommendedWatcher` (non-recursive) and a `HashMap<VfsUri, refcount>`. `fm-vfs` and the UI never see `notify` types. The watcher runs on notify's own thread; its callback only sends a normalized `uri` over a channel.
- `Session::poll` drains the channel, coalesces per URI inside a 100 ms window, and calls `DirCache::invalidate`; the existing re-read path submits the `ReadDir` job on the `local` lane. Events for URIs not currently cached or displayed are dropped.
- Reconciliation: after each layout/navigation change, `Session` computes the set of visible local directories and diffs it against the watched set (add/remove). Hidden workspace tabs are unwatched.
- Windows: the `ReadDirectoryChangesW` buffer can overflow; `notify` reports this as a rescan event, handled as a full invalidate. Paths are normalized (drive-letter case, `\\?\` prefix, separators) before mapping to `VfsUri`. Network drives/UNC paths are classified "non-event" and use polling.
- macOS: FSEvents latency adds coalescing; the same debounce applies. Linux: `ENOSPC` on watch creation (inotify limit) yields a status notice and polling for that directory.
- Poll fallback: remotes keep the existing 5 s visible poll plus focus-regain refresh; local folders without a live watch (network filesystems, UNC, over the cap, watch failure) poll every 2 s. Polls re-read through the job lane and `DirCache::refresh` ignores identical entries, so an unchanged directory does not bump the revision or redraw.
- Echo suppression: ops already invalidate on completion; a watcher invalidation arriving within the debounce window of an op-driven one is merged by the coalescer.
- Selection/scroll preservation is done by the pane when the revision changes: reselect by name, clamp scroll.

## Risks

- Event floods (build directories): debounce plus single in-flight read per URI (existing cache guarantee) bounds the work.
- Events with more than four paths mark all active watches for rescan rather than discarding paths beyond the inspection cap. The callback still allocates no per-event queue.
- Failed watcher initialization and failed directory watches retry at 1/2/4/8/16/32/60-second intervals, capped at 60 seconds; polling continues while waiting. A successful retry clears its backoff. Directory retries are removed when no longer visible; network paths remain polling-only, and over-cap paths are reconsidered when a slot is freed. Notices are emitted on the first failure of a retry sequence, not every retry.
- Symlinked directories: watch the resolved path but key by the displayed URI.
- Platform behaviour differs; Windows and macOS need real-machine smoke checks in addition to fixture tests.
