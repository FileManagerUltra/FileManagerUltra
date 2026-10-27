# terminal-pane Specification

## Purpose
TBD - created by archiving change pane-types. Update Purpose after archive.
## Requirements
### Requirement: Terminal session lifecycle
A Terminal pane SHALL own one pty-backed shell session, started on first show in the pane's directory when that is a local path, otherwise in the user's home directory. The shell is the user's default shell (the platform default program: `$SHELL`, `%COMSPEC%` on Windows) with `TERM=xterm-256color`. Closing the pane or quitting SHALL terminate the child; no orphan process remains.

#### Scenario: Start in pane directory
- **WHEN** a Files pane at a local directory `/tmp/x` switches to Terminal
- **THEN** the shell's working directory is `/tmp/x`

#### Scenario: Close terminates child
- **WHEN** a Terminal pane is closed
- **THEN** the child process has exited and its pty is closed

### Requirement: Non-blocking I/O
Reading and writing the pty SHALL happen off the UI thread; each frame the UI only drains an already-buffered, size-bounded queue. Output flooding (e.g. `yes`) MUST NOT stall frames.

#### Scenario: Flood
- **WHEN** the child prints continuously
- **THEN** each frame processes at most a bounded number of bytes and input stays responsive

### Requirement: Rendering
The terminal SHALL render the visible grid (text, 16/256/true colour, bold/italic/underline/inverse, cursor) with a monospace font and theme-token default colours, in both dark and light. Cells are virtualised to the visible size; resizing the pane resizes the pty (rows/cols) with a minimum of 2×2.

#### Scenario: Resize
- **WHEN** the pane is resized
- **THEN** the pty and parser grid are resized to the new rows/cols

#### Scenario: Line-drawing boxes
- **WHEN** a curses/dialog program designates the DEC Special Graphics set (`ESC ( 0`, or `ESC ) 0` + SO) and prints `lqqk`/`x`/`mqqj`
- **THEN** the cells hold `┌──┐`/`│`/`└──┘`, and light box characters are painted edge to edge so borders join without gaps

### Requirement: Input routing
While a Terminal pane is focused, text, Enter, Backspace, Tab, arrows, Home/End, PageUp/Down, Delete, F1–F12 and `Ctrl+<letter>` SHALL be sent to the shell. Only `Ctrl+Shift+…` chords and `Ctrl+Shift+Tab` are handled as app shortcuts. Mouse wheel scrolls scrollback; typing returns to the live screen.

#### Scenario: Ctrl+C
- **WHEN** Ctrl+C is pressed in a focused terminal
- **THEN** byte 0x03 is sent to the shell and no app command runs

#### Scenario: Unfocused pane
- **WHEN** another pane is focused
- **THEN** key events are not sent to the terminal

### Requirement: Copy, paste, scrollback
`Ctrl+Shift+C` SHALL copy the selection and `Ctrl+Shift+V` SHALL paste (bracketed when the app enabled it). Scrollback MUST be capped (10 000 lines). Paste larger than 1 MiB is rejected with a notice.

#### Scenario: Oversized paste
- **WHEN** more than 1 MiB is pasted
- **THEN** nothing is sent and a notice explains why

### Requirement: Shell exit and restart
When the shell exits the pane SHALL show "process exited (code N)" and offer `term.restart`; it MUST NOT close the pane or crash.

#### Scenario: Exit
- **WHEN** the user types `exit`
- **THEN** the exit banner appears and `term.restart` starts a fresh shell

### Requirement: Hostile output
Escape sequences are untrusted. The parser MUST bound line width, title/OSC lengths and scrollback, ignore unsupported sequences, and never execute or open anything from output (no hyperlink launching, no clipboard-write sequences honoured, no title used unescaped).

#### Scenario: Garbage
- **WHEN** the child emits random bytes and oversized OSC strings
- **THEN** the session stays alive, memory stays bounded and no panic occurs

### Requirement: Terminal commands
`term.copy`, `term.paste`, `term.cd_here`, `term.clear_scrollback` and `term.restart` SHALL be registered commands, enabled only when the focused pane is a Terminal, with bad params returning `Err`. `term.cd_here` sends `cd` to the other Files pane's local directory, shell-quoted.

#### Scenario: cd_here quoting
- **WHEN** the other pane is `/tmp/a b'c`
- **THEN** the shell receives a correctly quoted `cd` and the name cannot inject commands

### Requirement: Close on shell exit
A terminal pane whose shell exits (exit, Ctrl-D) SHALL close. The last remaining pane stays and shows the restart banner.

#### Scenario: Exit closes the pane
- **WHEN** the shell in a terminal pane exits and other panes exist
- **THEN** the pane is closed

