## MODIFIED Requirements

### Requirement: Quick filter and selection commands
Typing in a focused Files listing SHALL open a filter bar above the listing with the typed text (a leading space does not open it) and put the cursor in it; `view.filter {text?, focus?}` does the same from the menu. The bar narrows the rows to names containing the text, or matching it when it has wildcards (`* ? [..]`); the filter is transient, cleared by Esc, the bar's clear button (`view.filter.clear`) or navigating away, and the selection is pruned to visible rows. Typing SHALL NOT open the location bar; Ctrl+L or clicking the path does. `edit.select_all` (Ctrl+A), `edit.select_none` (Ctrl+Shift+A), `edit.invert_selection` (Ctrl+Shift+I) and `edit.select_pattern {pattern, mode?}` (Ctrl+Shift+S, a dialog) SHALL act on the visible rows only.

#### Scenario: Filter then select all
- **WHEN** the user types `o` in the listing and presses Ctrl+A
- **THEN** only the visible matching rows are selected

#### Scenario: Typing a path-like text
- **WHEN** the user types `/etc` in a focused listing
- **THEN** the listing is filtered by `/etc` and the location bar does not open
