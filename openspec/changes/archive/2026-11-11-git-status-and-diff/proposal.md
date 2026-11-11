# Git status markers and a Diff view in the Preview pane

Read-only git integration for local folders: show each file's and folder's git status in Files
panes and the tree, and compare a file with any revision in a **Diff** view of the Preview pane,
with line-level highlighting.

## Why

Developers browse project trees in a file manager and want to see what changed without
switching to a terminal. The Preview pane already follows the selection, so showing the
changes there is the natural place.

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
- **Diff view:** the Preview pane gets Preview/Diff buttons for a single local file in a
  repository; modified files open in Diff. Two aligned columns, line-level add/remove/change
  highlighting, intra-line highlight, next/previous change (F7/Shift+F7), a unified fallback in
  narrow panes, and a revision picker (HEAD, recent, local/remote branches, tags). The choice
  and revision stay with the pane as the selection moves.
- **Settings:** `prefs.git.enabled` (default on) and marker style toggles (color, letter).
- **Non-goals (v1):** stage/unstage/commit/restore/checkout, merge tooling, blame, history
  browser, remote operations, SFTP/SMB repositories, submodule recursion, editing in the Diff
  view, comparing two arbitrary files.

## Capabilities

- New: `git-status`, `git-diff`
- Modified: `preview-pane` (Diff view), `security` (hostile repositories, limits)
