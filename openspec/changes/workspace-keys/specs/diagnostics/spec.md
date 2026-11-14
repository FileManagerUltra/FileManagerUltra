## ADDED Requirements

### Requirement: Shortcut chords in the diagnostic log
While the diagnostic log is on, each key press with Ctrl, Cmd or Alt held SHALL be logged as a `key` event naming the modifiers, the key egui received, the physical key when it differs, and the command id it ran or `unmatched`. Key presses without those modifiers, typed text and pasted text SHALL NOT be logged.

#### Scenario: Finding a dead shortcut
- **WHEN** the log is on and the user presses Ctrl+Shift+X
- **THEN** the log shows `key: Ctrl+Shift+X -> pane.zoom` (or `unmatched`), so a shortcut that never reaches its command can be told apart from one that does

#### Scenario: Typing stays private
- **WHEN** the log is on and the user types a password or a file name
- **THEN** nothing of it appears in the log
