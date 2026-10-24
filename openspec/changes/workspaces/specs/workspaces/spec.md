## ADDED Requirements

### Requirement: Workspace tabs
The window SHALL show a tab bar of open workspaces. Each workspace owns its full pane layout; switching tabs MUST NOT disturb the others, and background terminals keep running. Closing the last tab replaces it with a fresh default.

#### Scenario: Switch keeps state
- **WHEN** the user changes a pane in tab 1, opens tab 2 and returns to tab 1
- **THEN** tab 1's layout is unchanged and pane ids never clash across tabs

#### Scenario: Shortcuts
- **WHEN** Ctrl+1..9/0 is pressed
- **THEN** the Nth open tab becomes active; Ctrl+T / Ctrl+W add / close, and are ignored while a terminal pane has focus

#### Scenario: Bad params
- **WHEN** `workspace.switch {n: 0}` or an out-of-range tab runs
- **THEN** the command returns an error and nothing changes

### Requirement: Saved workspaces
At most 10 workspaces SHALL be saved, with unique case-insensitive names of 1–40 characters without control characters. Each has Auto-save, Open at startup and Default flags. Auto-save writes changes back continuously; otherwise only `workspace.save` writes.

#### Scenario: Cap
- **WHEN** an 11th workspace is saved
- **THEN** the command fails with an actionable error

### Requirement: Startup restore
On start, saved workspaces flagged Open at startup SHALL open; else the Default one; else a built-in single pane. Corrupt, oversized or invalid entries MUST be skipped with a notice and never crash.

#### Scenario: Round trip
- **WHEN** a saved workspace is written and the app restarts
- **THEN** its panes, locations and per-pane settings are restored

### Requirement: Unsaved changes prompt
Quitting or closing a non-auto-save workspace with unsaved changes (or a never-saved user tab) SHALL prompt Save / Discard / Cancel. Auto-save workspaces never prompt.

#### Scenario: Cancel
- **WHEN** the user cancels the quit prompt
- **THEN** the app keeps running with all tabs intact

### Requirement: Persistence is private
`workspaces.json` SHALL contain no credentials, SHALL be written atomically with owner-only permissions, and SHALL be size-capped when read.

#### Scenario: Hostile file
- **WHEN** `workspaces.json` is oversized, malformed, or contains duplicate or oversize pane trees
- **THEN** the offending entries are skipped, a notice is shown, and the app starts normally
