## Implementation

- [x] `fm-git` crate (gix), layer registration, repo discovery incl. worktrees.
- [x] Status map + folder aggregation, caps, LRU, single-flight refresh on the local lane.
- [x] Refresh triggers: live-watch invalidation, focus regain, `.git/index`/`HEAD` stat short circuit.
- [x] Theme tokens (dark/light), gutter/tint rendering in listing and tree, branch in footer.
- [x] Prefs (`git.enabled`, marker toggles) and `git.*` commands with bad-param tests.
- [x] Blob loading, diff model (`similar`), caps and binary detection.
- [x] `PaneKind::Diff`: layout state + serde, kind switcher, `diff.*` commands.
- [x] Diff widget: aligned columns, synced scroll, intra-line highlight, next/prev change, narrow fallback.
- [x] Entry points: context menu Compare with HEAD / Compare with…, drag onto a side.
- [x] Security tests: hostile repo config, huge repo, symlink loops, caps; command fuzzing.
- [x] Snapshots (dark/light), temp-repo integration tests, performance check on a large repo.
- [x] Docs: architecture, roadmap, development smoke checks, specs; devlog.
