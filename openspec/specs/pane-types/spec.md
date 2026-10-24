# pane-types Specification

## Purpose
TBD - created by archiving change pane-types. Update Purpose after archive.
## Requirements
### Requirement: Pane kinds
Every pane SHALL have exactly one `PaneKind` from `Files`, `Terminal`, `Sftp`, `Network`; new panes are `Files`. The kind is view state in `fm-layout` and changes only through the `pane.kind.set {kind}` command.

#### Scenario: Switch a pane to a terminal
- **WHEN** `pane.kind.set {"kind":"terminal"}` runs on the focused pane
- **THEN** that pane's kind is `Terminal` and other panes are unchanged

#### Scenario: Unknown kind
- **WHEN** `pane.kind.set` runs with an unknown kind, a missing or non-string `kind`, or non-object params
- **THEN** it returns an error and the pane is unchanged

### Requirement: Leaving a terminal ends it
Switching a pane away from `Terminal` SHALL terminate its shell and everything running in it. When a command other than the shell owns the terminal (where the platform can tell), `pane.kind.set` MUST NOT switch until the user confirms (`confirm: true`); the UI shows a confirmation and cancelling leaves the terminal untouched.

#### Scenario: Idle terminal
- **WHEN** a terminal with only its shell running switches to `Files`
- **THEN** the shell is terminated and the pane is `Files`, with no prompt

#### Scenario: Busy terminal
- **WHEN** a terminal running `sleep 30` switches to `Files` without `confirm`
- **THEN** the pane stays `Terminal`, a confirmation is pending, and nothing is killed
- **WHEN** the switch is repeated with `confirm: true`
- **THEN** the terminal and its processes are terminated and the pane is `Files`

### Requirement: Files state survives a kind switch
Switching a pane away from `Files` and back SHALL restore its location, history, selection, sort and tree state unchanged.

#### Scenario: Round trip
- **WHEN** a Files pane at `/a/b` with a selection switches to `Terminal` and back to `Files`
- **THEN** it shows `/a/b` with the same selection and history

### Requirement: Placeholder kinds
`Sftp` and `Network` SHALL appear in the selector and command but be disabled until `network-backends` lands; running the command for them returns an actionable error and never changes the pane.

#### Scenario: Selecting Sftp
- **WHEN** `pane.kind.set {"kind":"sftp"}` runs
- **THEN** it returns an error naming the missing backend and the pane keeps its kind

### Requirement: Kind selector
Each pane header SHALL offer a kind selector (and View ▸ Pane Type menu) that dispatches `pane.kind.set`; the UI MUST NOT mutate the kind directly. Colours come from theme tokens and render in dark and light.

#### Scenario: Selector dispatches the command
- **WHEN** the user picks Terminal in the selector
- **THEN** `pane.kind.set` is dispatched for that pane

### Requirement: Kind-aware commands
File commands (`file.*`, `nav.*`, drag & drop) SHALL be disabled when the focused pane is not `Files`; a non-Files pane is never a drop target or an F5/F6 destination.

#### Scenario: Copy with a terminal focused
- **WHEN** a Terminal pane is focused and `file.copy` is queried
- **THEN** it is disabled (running it anyway is a no-op)

### Requirement: Preview is a supported pane kind
`PaneKind::Preview` SHALL appear in the type switcher and be accepted by `pane.kind.set`; switching to it SHALL not change any Files pane, and switching away SHALL stop any playback.

#### Scenario: Switch to preview
- **WHEN** `pane.kind.set {kind: "preview"}` runs on a Files pane
- **THEN** the pane becomes a Preview following the most recently focused other Files pane

