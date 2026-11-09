## ADDED Requirements

### Requirement: Media pane kind
`PaneKind` SHALL include `Media`, selectable through `pane.kind.set {"kind":"media"}` and the pane kind selector. Switching a pane to `Media` SHALL NOT start playback by itself and SHALL keep the session queue; switching away SHALL stop video and, unless background playback is on, audio. A saved workspace containing a Media pane SHALL restore it empty-state (queue from persisted state, paused). Files state SHALL survive a round trip through `Media` as for other kinds.

#### Scenario: Switch to media
- **WHEN** `pane.kind.set {"kind":"media"}` runs on the focused pane
- **THEN** its kind is `Media`, nothing starts playing, and other panes are unchanged

#### Scenario: Round trip
- **WHEN** a Files pane at `/a/b` with a selection switches to `Media` and back
- **THEN** it shows `/a/b` with the same selection and history

#### Scenario: Leave while playing video
- **WHEN** a Media pane playing video switches to `Files`
- **THEN** the video decoder process no longer exists
