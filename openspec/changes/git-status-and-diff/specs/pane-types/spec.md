## MODIFIED Requirements

### Requirement: Pane kinds
Every pane SHALL have exactly one `PaneKind` from `Files`, `Terminal`, `Sftp`, `Network`, `Preview`, `Diff`; new panes are `Files`. The kind is view state in `fm-layout` and changes only through the `pane.kind.set {kind}` command.

#### Scenario: Switch a pane to a terminal
- **WHEN** `pane.kind.set {"kind":"terminal"}` runs on the focused pane
- **THEN** that pane's kind is `Terminal` and other panes are unchanged

#### Scenario: Switch a pane to diff
- **WHEN** `pane.kind.set {"kind":"diff"}` runs on a pane that has no comparison
- **THEN** it returns an error ("open a comparison with diff.open first") and the pane is unchanged

#### Scenario: Diff from the pane kind menu
- **WHEN** the user chooses Diff in a pane's kind menu
- **THEN** the pane's selection is compared (two files, or one file against HEAD); with neither a notice explains what to select

#### Scenario: Unknown kind
- **WHEN** `pane.kind.set` runs with an unknown kind, a missing or non-string `kind`, or non-object params
- **THEN** it returns an error and the pane is unchanged
