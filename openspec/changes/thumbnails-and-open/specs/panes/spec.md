## ADDED Requirements

### Requirement: Panes have a close button
Every pane header SHALL show a close button while more than one pane exists, dispatching `pane.close {pane}`. `pane.close` SHALL accept an optional `pane` id (default: the focused pane) and reject a non-numeric or unknown id with `Err` and no change.

#### Scenario: Close by id
- **WHEN** `pane.close {"pane": <id>}` is dispatched with three panes open
- **THEN** exactly that pane is closed and the other two remain
