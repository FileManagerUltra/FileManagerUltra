## MODIFIED Requirements

### Requirement: Terminal session lifecycle
A Terminal pane SHALL own one pty-backed shell session, started on first show in the first local folder of: the active pane (a Files pane's folder, or a terminal's current folder; a pane being switched to Terminal counts as the Files pane it was), the pane active just before it (for a split, the terminal it was split from), the most recently active Files pane's folder in any workspace, and finally the user's home directory. A terminal pane's stored location SHALL NOT be used. A terminal's current folder SHALL be the shell's working directory where the OS exposes it without FFI (Linux), otherwise the folder it started in or was last sent to by `term.cd_here`. `term.restart` SHALL start the new shell in the old shell's current folder. The shell follows the `terminal_shell` preference (`prefs.terminal.shell`, applied to new sessions): `default` is PowerShell on Windows (PowerShell 7 `pwsh` from `PATH`, else the built-in Windows PowerShell, else `%COMSPEC%`) and the login shell (`$SHELL`) elsewhere; `powershell` and `cmd` select those explicitly and fail with an error where they do not exist. It runs with `TERM=xterm-256color`. Closing the pane or quitting SHALL terminate the child; no orphan process remains.

#### Scenario: Start in pane directory
- **WHEN** a Files pane at a local directory `/tmp/x` switches to Terminal
- **THEN** the shell's working directory is `/tmp/x`

#### Scenario: Split a terminal
- **WHEN** the user runs `cd /tmp/y` in a terminal on Linux and splits that terminal
- **THEN** the new terminal starts in `/tmp/y`, whatever the pane's stored location or the workspace's Files panes say

#### Scenario: Workspace without Files panes
- **WHEN** the user last worked in a Files pane at `/home/alice` in workspace 1 and opens a terminal in workspace 2, which has only terminals that have never been told a folder
- **THEN** the terminal starts in `/home/alice`

#### Scenario: PowerShell on Windows
- **WHEN** a Windows user opens a Terminal pane with the default shell setting
- **THEN** PowerShell starts; choosing `cmd` in Preferences makes new terminals start cmd.exe

#### Scenario: Close terminates child
- **WHEN** a Terminal pane is closed
- **THEN** the child process has exited and its pty is closed
- **AND** the remainder of that frame does not render or restart the removed pane or report a stale pane-not-found error
