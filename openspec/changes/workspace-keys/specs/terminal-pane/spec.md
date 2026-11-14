## MODIFIED Requirements

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
