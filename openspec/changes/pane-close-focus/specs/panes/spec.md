## MODIFIED Requirements

### Requirement: Panes can be split, closed, focused and swapped by command
All pane topology changes SHALL be engine commands; the UI never mutates the layout tree directly.

#### Scenario: Closing a pane keeps the tree valid
- **WHEN** `pane.close` runs with two or more panes
- **THEN** the focused leaf is removed and its sibling inherits the parent slot
- **AND** if the closed pane was focused and its previous-focus pane still exists, that pane becomes focused
- **AND** otherwise a surviving pane becomes focused

#### Scenario: Closing a non-focused pane keeps focus
- **WHEN** `pane.close {pane}` closes a pane other than the focused pane
- **THEN** the currently focused pane remains focused unless it is no longer available

#### Scenario: The last pane cannot be closed
- **WHEN** `pane.close` runs with exactly one pane
- **THEN** the command's `enabled` predicate is false (menu greyed) and dispatch is a no-op that returns `Ok` without changing the layout
