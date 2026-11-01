# Git status markers and side-by-side Diff pane

Read-only git integration for local folders: show each file's and folder's git status in Files
panes and the tree, and compare any two files or revisions side by side in a new **Diff** pane
type with line-level highlighting.

## Why

Developers browse project trees in a file manager and want to see what changed without
switching to a terminal. Two linked Preview panes show raw content but no differences, so
comparison needs its own pane type.

## Scope

- **Status:** detect the enclosing repository of any local folder shown by a Files pane (walk
  up to `.git`, including worktrees; bounded depth). Show per entry a colored name plus a small
  status letter in a gutter column: Modified, Added, Removed, Renamed, Untracked, Ignored,
  Conflicted. Folders aggregate: a "contains changes" marker (highest-priority child status;
  ignored folders are dimmed, not aggregated). The tree uses the same colors/markers. The
  current branch appears in the pane footer/status.
- **Backend:** `gix` (pure Rust, no git binary, no `unsafe`, no FFI). New crate `fm-git` (L0)
  wraps it behind a small API returning plain data. No hooks, filters, credential helpers or
  external commands are ever run.
- **Diff pane:** new `PaneKind::Diff` with two aligned columns, line-level add/remove/change
  highlighting, intra-line highlight for changed lines, synced scrolling, next/previous change
  navigation, and a unified fallback in narrow panes. Inputs are any two *sources*: a working
  file, a file at a revision (HEAD, index, branch, tag, commit), or another file on disk.
  Entry points: context menu "Compare with HEAD" / "Compare with…", the `diff.open {left,
  right}` command, and dragging a file onto a Diff pane side.
- **Settings:** `prefs.git.enabled` (default on) and marker style toggles (color, letter).
- **Non-goals (v1):** stage/unstage/commit/restore/checkout, merge tooling, blame, history
  browser, remote operations, SFTP/SMB repositories, submodule recursion, editing in the Diff
  pane.

## Capabilities

- New: `git-status`, `git-diff`
- Modified: `pane-types` (Diff kind), `security` (hostile repositories, limits)
