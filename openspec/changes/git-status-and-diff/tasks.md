## Implementation

- [x] `fm-git` crate (gix), layer registration, repo discovery incl. worktrees.
- [x] Status map + folder aggregation, caps, LRU, single-flight refresh on the local lane.
- [x] Refresh triggers: live-watch invalidation, focus regain, `.git/index`/`HEAD` stat short circuit.
- [x] Theme tokens (dark/light), gutter/tint rendering in listing and tree, branch in footer.
- [x] Prefs (`git.enabled`, marker toggles) and `git.*` commands with bad-param tests.
- [x] Blob loading, diff model (`similar`), caps and binary detection.
- [x] Diff as a Preview pane view: `preview.diff.*` commands, default for modified files, sticky choice and revision.
- [x] Diff widget: aligned columns, intra-line highlight, next/prev change (F7/Shift+F7), narrow fallback, options menu.
- [x] Revision picker: search, recent/local/remote/tags, resizable popup (`fm_git::refs`).
- [x] Security tests: hostile repo config, huge repo, symlink loops, caps; command fuzzing.
- [x] Snapshots (dark/light), temp-repo integration tests, performance check on a large repo.
- [x] Docs: architecture, roadmap, development smoke checks, specs; devlog.
