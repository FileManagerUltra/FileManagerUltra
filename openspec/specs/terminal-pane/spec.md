# terminal-pane Specification

## Purpose
TBD - created by archiving change pane-types. Update Purpose after archive.
## Requirements
### Requirement: Terminal session lifecycle
A Terminal pane SHALL own one pty-backed shell session, started on first show in the first local folder of: the active pane (a Files pane's folder, or a terminal's current folder; a pane being switched to Terminal counts as the Files pane it was), the pane active just before it (for a split, the terminal it was split from), the most recently active Files pane's folder in any workspace, and finally the user's home directory. A terminal pane's stored location SHALL NOT be used; a terminal restored from a saved workspace SHALL instead start once in the folder its shell was in when the workspace was saved, when that folder still exists. A terminal's current folder SHALL be the shell's working directory where the OS exposes it without FFI (Linux), otherwise the folder the shell last reported, otherwise the folder it started in or was last sent to by `term.cd_here`. A reported folder is an OSC 7 (`file://host/path`, percent-decoded) or OSC 9;9 (`"C:\path"`) sequence; it SHALL be UTF-8 without control characters, absolute and at most 4096 bytes, and is used only while it exists. So that Windows shells report their folder, PowerShell SHALL start with `-NoExit -EncodedCommand` wrapping the prompt it already has (from its profile) to send OSC 9;9 for file-system locations, and cmd SHALL get `$e]9;9;$P$e\` in front of its `PROMPT` (the user's own, or `$P$G`). `term.restart` SHALL start the new shell in the old shell's current folder. The shell follows the `terminal_shell` preference (`prefs.terminal.shell`, applied to new sessions): `default` is PowerShell on Windows (PowerShell 7 `pwsh` from `PATH`, else the built-in Windows PowerShell, else `%COMSPEC%`) and the login shell (`$SHELL`) elsewhere; `powershell` and `cmd` select those explicitly and fail with an error where they do not exist. It runs with `TERM=xterm-256color`. Closing the pane or quitting SHALL terminate the child; no orphan process remains.

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

### Requirement: Non-blocking I/O
Reading and writing the pty SHALL happen off the UI thread; each frame the UI only drains an already-buffered, size-bounded queue. Output flooding (e.g. `yes`) MUST NOT stall frames.

#### Scenario: Flood
- **WHEN** the child prints continuously
- **THEN** each frame processes at most a bounded number of bytes and input stays responsive

### Requirement: Rendering
The terminal SHALL render the visible grid (text, 16/256/true colour, bold/italic/underline/inverse, cursor) with a monospace font and theme-token default colours, in both dark and light. Cells are virtualised to the visible size; resizing the pane resizes the pty (rows/cols) with a minimum of 2×2.

Width changes SHALL reflow normal-screen content and retained scrollback at character-cell
boundaries, joining soft wraps but preserving explicit line breaks, attributes and
wide/combining characters. The cursor and saved cursor SHALL follow their content;
an existing scrolled view SHALL remain anchored to its logical line while retained.
History remains bounded to the session's configured visual-row limit; narrowing may evict oldest history.
As with height resizing, excess bottom rows may be discarded to keep the cursor visible.
Alternate-screen applications SHALL NOT be reflowed: they redraw after the pty resize.
Existing text selections may be cleared when grid dimensions change.

#### Scenario: Resize
- **WHEN** the pane is resized
- **THEN** the pty and parser grid are resized to the new rows/cols

#### Scenario: Narrow and widen shell output
- **WHEN** long normal-screen output is narrowed and widened again within the retained-history limit
- **THEN** soft-wrapped text and scrollback reflow without losing characters or merging hard newlines, and subsequent typing continues at the mapped cursor

#### Scenario: Resize a full-screen application
- **WHEN** an alternate-screen application such as vim is resized
- **THEN** its grid is resized without reflow and the application can redraw; the hidden normal buffer reflows for restoration on exit

#### Scenario: Line-drawing boxes
- **WHEN** a curses/dialog program designates the DEC Special Graphics set (`ESC ( 0`, or `ESC ) 0` + SO) and prints `lqqk`/`x`/`mqqj`
- **THEN** the cells hold `┌──┐`/`│`/`└──┘`, and light box characters are painted edge to edge so borders join without gaps

### Requirement: Input routing
Preferences and other open forms, or a focused text editor, SHALL own keyboard and
clipboard input ahead of an underlying Terminal pane. Copy/cut/paste in a dialog
SHALL NOT send control bytes or pasted text to the shell.

While a Terminal pane is focused, text, Enter, Backspace, Tab, arrows, Home/End, PageUp/Down, Delete, F1–F12 and `Ctrl+<letter>` SHALL be sent to the shell. Only `Ctrl+Shift+…` chords, `Ctrl+Tab` and `Ctrl+Shift+Tab` (switching workspace tabs) are handled as app shortcuts. Mouse wheel scrolls scrollback unless the application requested mouse reporting; typing returns to the live screen.

#### Scenario: Ctrl+C
- **WHEN** Ctrl+C is pressed in a focused terminal
- **THEN** byte 0x03 is sent to the shell and no app command runs

#### Scenario: Unfocused pane
- **WHEN** another pane is focused
- **THEN** key events are not sent to the terminal

#### Scenario: Switching workspaces from a terminal
- **WHEN** Ctrl+Tab or Ctrl+Shift+Tab is pressed in a focused terminal
- **THEN** the workspace tab changes as elsewhere and nothing is sent to the shell; panes are no longer swapped by Ctrl+Shift+Tab

### Requirement: Application mouse input
When a terminal application requests mouse input, pointer events SHALL be sent through `term.mouse`, using the application's requested protocol: X10 (`?9`), press/release (`?1000`), button motion (`?1002`) or any motion (`?1003`), with default byte, UTF-8 (`?1005`) or SGR (`?1006`) encoding. Reports SHALL contain one-based screen-cell coordinates, left/middle/right buttons, wheel up/down and modifiers; movement is reported only when the cell changes and the requested mode permits it. Legacy bytes MUST NOT be converted to UTF-8; coordinates outside a legacy encoding's range are not reported at a different location.

Holding Shift SHALL bypass application mouse input for local text selection and scrollback. Without application mouse reporting, existing local mouse behavior SHALL be preserved. Applications receive no clicks originating outside their pane or under a popup. A captured button SHALL be released even if the pointer leaves the pane/window or Shift is subsequently held. Wheel accumulation SHALL retain fractional steps and emit at most 32 reports per event. Disabling/resetting mouse mode restores local behavior. Mouse input SHALL use the bounded asynchronous terminal write queue.

#### Scenario: Click and scroll in a terminal application
- **WHEN** mc or btop enables mouse reporting and the user clicks a menu item or turns the wheel over the terminal
- **THEN** the application receives the corresponding button or wheel reports at that cell, rather than a local text selection or scrollback action

#### Scenario: Drag outside the pane
- **WHEN** a button is pressed in an application with button-motion reporting and dragged outside the pane
- **THEN** movement uses bounded screen coordinates and the eventual release reaches the application

#### Scenario: Select text while mouse reporting is active
- **WHEN** the user holds Shift and drags or scrolls over a terminal application
- **THEN** local selection or scrollback is used and no new application mouse gesture starts

#### Scenario: Malformed mouse command
- **WHEN** `term.mouse` receives an unknown action/button, wrong-typed modifier, negative/oversized coordinate or a cell outside the current grid
- **THEN** it returns an error without writing to the shell or panicking

### Requirement: Copy, paste, scrollback
`Ctrl+Shift+C` SHALL copy the selection and `Ctrl+Shift+V` SHALL paste (bracketed when the app enabled it). Scrollback MUST be capped by the persisted `terminal_scrollback_rows` preference: integer values from 5 000 through 1 000 000 visual rows, default 10 000, excluding the visible screen. Preferences > Terminal SHALL expose this through `prefs.set` and explain the memory cost of large histories. Rows SHALL allocate as output arrives, not upfront. Changes SHALL apply only to new or restarted terminals, leaving running sessions unchanged. Invalid command values SHALL fail without changing preferences; hand-edited out-of-range values SHALL be bounded when used. Paste larger than 1 MiB is rejected with a notice.

#### Scenario: Configure terminal history
- **WHEN** the user changes scrollback to 5 000 or 1 000 000 rows
- **THEN** the value persists and new or restarted terminals use that limit
- **AND** already-running terminals retain their previous limit
- **WHEN** the command receives 4 999, 1 000 001, a fraction or a non-number
- **THEN** it returns an error and preserves the previous setting

Selections SHALL be anchored in retained buffer rows, not viewport rows. Mouse
wheel and Shift+PageUp/PageDown SHALL preserve selection, and dragging near the
top/bottom edge SHALL scroll to extend it beyond one viewport. Shift-click SHALL
extend an existing selection. Copy SHALL include off-screen selected text without
moving the view. Application mouse reporting SHALL retain its existing Shift
bypass. Resizing, buffer resets or eviction of selected history MAY clear selection
to avoid copying different text.

#### Scenario: Copy more than one screen
- **WHEN** the user drags a selection, scrolls into history and extends it
- **THEN** the anchor stays on its original buffer text and copying includes every selected row
- **AND** scrolling alone neither clears nor relocates the selection

#### Scenario: Oversized paste
- **WHEN** more than 1 MiB is pasted
- **THEN** nothing is sent and a notice explains why

### Requirement: Shell exit and restart
When the shell exits the pane SHALL show "process exited (code N)" and offer `term.restart`; it MUST NOT close the pane or crash.

#### Scenario: Exit
- **WHEN** the user types `exit`
- **THEN** the exit banner appears and `term.restart` starts a fresh shell

### Requirement: Terminal queries
The emulator SHALL answer cursor-position (`CSI 6 n`), status (`CSI 5 n`) and device-attribute (`CSI c`, `CSI > c`) requests in stream order, with the cursor position as of the request, and the replies SHALL be bounded so a query flood cannot grow memory.

#### Scenario: Windows ConPTY start-up
- **WHEN** a shell starts on Windows and ConPTY asks for the cursor position before drawing
- **THEN** the request is answered and the prompt appears

### Requirement: Hostile output
Escape sequences are untrusted. The parser MUST bound line width, title/OSC lengths and scrollback, ignore unsupported sequences, and never execute or open anything from output (no hyperlink launching, no clipboard-write sequences honoured, no title used unescaped).

#### Scenario: Garbage
- **WHEN** the child emits random bytes and oversized OSC strings
- **THEN** the session stays alive, memory stays bounded and no panic occurs

### Requirement: Terminal commands
`term.copy`, `term.paste`, `term.cd_here`, `term.clear_scrollback` and `term.restart` SHALL be registered commands, enabled only when the focused pane is a Terminal, with bad params returning `Err`. `term.cd_here` sends `cd` to the other Files pane's local directory, quoted for the running shell's dialect (POSIX `cd -- '…'`, cmd `cd /d "…"`, PowerShell `Set-Location -LiteralPath '…'` with every single-quote form doubled), after clearing the typed line; dropped paths use the same quoting.

#### Scenario: PowerShell quoting
- **WHEN** `term.cd_here` targets `C:\it's $(calc)` in a PowerShell terminal
- **THEN** the shell receives `Set-Location -LiteralPath 'C:\it''s $(calc)'` and nothing is evaluated

#### Scenario: cd_here quoting
- **WHEN** the other pane is `/tmp/a b'c`
- **THEN** the shell receives a correctly quoted `cd` and the name cannot inject commands

### Requirement: Close on shell exit
A terminal pane whose shell exits (exit, Ctrl-D) SHALL close. The last remaining pane stays and shows the restart banner.

#### Scenario: Exit closes the pane
- **WHEN** the shell in a terminal pane exits and other panes exist
- **THEN** the pane is closed

### Requirement: Per-terminal zoom
Each terminal pane SHALL have its own text zoom from 50% to 400% in ×1.2 steps, applied on top of the terminal font size, independent of other terminals and of the interface zoom, and saved with the pane in its workspace. Its zoom controls SHALL be the pane header's zoom (requirement "Pane zoom" in `panes`); nothing SHALL float over the terminal's text. `term.zoom.in`, `term.zoom.out` and `term.zoom.reset` take `{pane?}` (a terminal pane, default the focused one) and have no shortcut of their own. A shortcut SHALL do the same in every pane: while a terminal has focus, Ctrl+Shift+=, Ctrl+Shift+- and Ctrl+Shift+0 SHALL keep changing the interface zoom of the whole application. A stored zoom that is not a finite number in range SHALL be read as bounded (100% when not a number).

#### Scenario: One terminal larger
- **WHEN** two terminals are open and the user opens the left one's header zoom and clicks + twice
- **THEN** the left terminal's text is 144% and re-flows to fewer columns and rows, the right one is unchanged, and the interface zoom and terminal font size preferences are unchanged

#### Scenario: Keyboard
- **WHEN** a terminal has focus and the user presses Ctrl+Shift+= then Ctrl+Shift+0
- **THEN** the whole application zooms in one step and back to 100%, as from any other pane; that terminal's own zoom is unchanged and nothing is sent to the shell

#### Scenario: Bad params
- **WHEN** a zoom command names a pane that is not a terminal, or receives malformed params
- **THEN** it returns an error and no zoom changes

