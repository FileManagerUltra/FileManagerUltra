## ADDED Requirements

### Requirement: Preview is a supported pane kind
`PaneKind::Preview` SHALL appear in the type switcher and be accepted by `pane.kind.set`; switching to it SHALL not change any Files pane, and switching away SHALL stop any playback.

#### Scenario: Switch to preview
- **WHEN** `pane.kind.set {kind: "preview"}` runs on a Files pane
- **THEN** the pane becomes a Preview following the most recently focused other Files pane
