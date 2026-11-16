# diagnostics Specification

## Purpose
The opt-in diagnostic log used to investigate problems on users' machines: what it records (renderer, memory and frame samples, commands, shortcut chords) and what it never records (typed text, paths, credentials).

## Requirements
### Requirement: Shortcut chords in the diagnostic log
While the diagnostic log is on, each key press with Ctrl or Cmd held SHALL be logged as a `key` event naming the modifiers, the key egui received, the physical key when it differs, and the command ids that frame ran or `unmatched`. Nothing that types text SHALL be logged: no key presses without Ctrl/Cmd, no Ctrl+Alt (AltGr on Windows), no Alt/Option-only chords, nothing while a text field has focus, and no typed or pasted text.

#### Scenario: Finding a dead shortcut
- **WHEN** the log is on and the user presses Ctrl+Shift+X
- **THEN** the log shows `key: Ctrl+Shift+X -> pane.zoom` (or `unmatched`), so a shortcut that never reaches its command can be told apart from one that does

#### Scenario: Typing stays private
- **WHEN** the log is on and the user types a password or a file name, including characters typed with AltGr or Option
- **THEN** nothing of it appears in the log

