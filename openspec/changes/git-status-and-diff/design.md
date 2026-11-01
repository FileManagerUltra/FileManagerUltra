## Architecture

- `crates/git` (`fm-git`, **L0**): `discover(path)`, `status(repo)`, `read_blob(repo, rev, path)`.
  Pure data out. Registered in `xtask/src/layers.rs`.
- `engine::git` (L3): per-repository state cache (repo root -> status map, branch, revision),
  refreshed on the `local` job lane; panes only read cached results. Commands in
  `engine/src/git_cmds.rs` and `diff_cmds.rs` (`git.refresh`, `git.markers.toggle`,
  `preview.diff.{toggle,picker,rev,next,prev,mode,eol,reload}`). `diff_cmds` keeps one
  `PaneDiff` per Preview pane, derived from the preview subject each frame, plus a sticky
  per-pane choice (view, revision, mode, EOL) that survives selection changes.
- `layout` (L2): session-only `DiffState { left, right, mode, ignore_eol, current, jumps }`;
  no new pane kind (saved workspaces with the earlier "Diff" kind load as Files).
- `ui` (L4): gutter column + name tint in listing/tree, the Diff view inside the Preview pane (`diff.rs`: bar, revision picker, rows). Colors only from
  `theme.rs` tokens (new `git_modified`, `git_added`, `git_removed`, `git_untracked`,
  `git_ignored`, `git_conflict`; dark and light exhaustive).

## Status computation and memory

- Status is computed per **repository** once, not per pane or directory: one compact map of
  changed entries only (clean files cost nothing) plus a folder-aggregate set. Cap: 200 000
  changed entries per repo, then truncated with a visible "status incomplete" notice.
- Refresh triggers: live-watch invalidation of a visible folder inside the repo, focus regain,
  `git.refresh`. Debounced (300 ms), single-flight per repo, never on the UI thread. A cheap stat
  of `.git/index` and `.git/HEAD` short-circuits rescans of unchanged repos.
- Only repos that a visible pane is inside are tracked; leaving a repo drops its state after a
  grace period; at most 8 repos cached (LRU).
- Untracked collection is ignore-aware (skips `node_modules`-style ignored trees) and capped.
- Blobs for diffs load on demand, capped (2 MiB per side), binary detected, and are dropped
  when the Preview pane closes or its file changes.

## Diff algorithm

Line diff with the `similar` crate (patience/Myers) over capped inputs; word-level diff only for
changed line pairs under a length cap. Computation runs on a worker; the result is an
`Arc<DiffModel>` of aligned rows (left?, right?, kind), rendered virtualized. UTF-8 with lossy
fallback; optional ignore-line-endings.

## Risks

- `gix` status speed on very large repos: single-flight, caps and the unchanged-index short
  circuit; measure on a 100k-file repo before archiving.
- Hostile repos: gix configuration can reference external programs (filters, fsmonitor, hooks).
  Open repos with options that neutralise them and never execute anything.
- Windows path/case handling and `core.autocrlf` must match git's view; fixtures cover it.
