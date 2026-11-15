## Context

`Session::ensure_terminal(pane)` starts a shell in `focus.last_files`'s local folder, else the pane's own `location`, else home. `last_files` lives in the per-workspace `Layout` and is set by `touch_files_focus` in `poll`. `layout.split` copies the source pane's state, so a pane split from a terminal carries that terminal's stored `location`, which is never updated while it is a terminal. `TermSession` does not know its shell's directory.

## Goals / Non-Goals

**Goals:** a new shell starts where the user was working: the active pane, the pane before it, the last Files folder in any workspace, home.

**Non-Goals:** parsing OSC 7 prompts; reading another process's directory through FFI on Windows/macOS.

## Decisions

1. **`TermSession::cwd()`**: on Linux `read_link("/proc/<pid>/cwd")` of the shell (std only, no FFI); elsewhere, and when that fails, the folder it was started in or last told via `note_cwd` (called by `term.cd_here`).
2. **Working folder of a pane** (`Session::pane_dir`): Files kinds → local folder of `location`; Terminal → `cwd()` when it is a local folder; other kinds → none.
3. **Order in `ensure_terminal(pane)`**: `focused` (if it is not `pane`, or `pane` is still a Files pane being switched), then `focus.previous` (if not `pane`), then `Session::last_files_dir`, then home.
4. **`Session::last_files_dir`**: updated in `poll` after `touch_files_focus` whenever the focused pane is a Files pane at a local folder; lives on `Session`, so it survives workspace switches.
5. **`term.restart`** reads the old session's `cwd()` before dropping it and starts the new one there (`ensure_terminal_in`).

## Risks / Trade-offs

- [On Windows/macOS a `cd` typed in the shell is not seen] → The split starts where that terminal started (or was last sent), which is still the right workspace context; Linux gets the exact folder.
