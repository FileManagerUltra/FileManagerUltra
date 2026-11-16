## Why

Splitting a terminal starts the new shell in an unexpected folder (for example `~/Pictures/Screenshots`) instead of where the user was working. A new shell starts in the last active Files pane's folder, else the pane's own stored location. Both are per workspace: in a workspace with only terminals there is no Files pane, and a split terminal inherits a stored location that is whatever the pane showed before it became a terminal, often long ago. The folder of the shell the user just split from is never considered.

## What Changes

- A new shell starts in the first local folder of:
  1. the active pane: a Files pane's folder, or a terminal's current folder (the pane being switched to Terminal counts as a Files pane at its folder);
  2. the pane that was active just before (for a split: the terminal it came from);
  3. the most recently active Files pane's folder in any workspace;
  4. the home folder.
- A terminal pane's stored location is no longer used as a start folder.
- A terminal's current folder is the shell's real working directory on Linux; on Windows and macOS (no FFI-free way to read another process's directory) it is the folder the shell started in, or where "cd to Other Pane" last sent it.
- Restart Shell starts the new shell in the folder the old one was in.

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `terminal-pane`: "Terminal session lifecycle" now defines the start folder as above.

## Impact

`fm-term` (`TermSession::cwd`, `note_cwd`), `fm-engine` (`term_cmds::ensure_terminal`, `term.restart`, `term.cd_here`, a session-wide last Files folder updated in `poll`). No new dependencies, no prefs change.
