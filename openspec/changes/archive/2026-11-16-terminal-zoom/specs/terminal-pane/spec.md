## ADDED Requirements

### Requirement: Per-terminal zoom
Each terminal pane SHALL have its own text zoom from 50% to 400% in ×1.2 steps, applied on top of the terminal font size, independent of other terminals and of the interface zoom, and saved with the pane in its workspace. Its zoom controls SHALL be the pane header's zoom (requirement "Pane zoom" in `panes`); nothing SHALL float over the terminal's text. `term.zoom.in`, `term.zoom.out` and `term.zoom.reset` take `{pane?}` (a terminal pane, default the focused one) and have no shortcut of their own. A shortcut SHALL do the same in every pane: while a terminal has focus, Ctrl+Shift+=, Ctrl+Shift+- and Ctrl+Shift+0 SHALL keep changing the interface zoom of the whole application. A stored zoom that is not a finite number in range SHALL be read as bounded (100% when not a number).

#### Scenario: One terminal larger
- **WHEN** two terminals are open and the user opens the left one's header zoom and clicks + twice
- **THEN** the left terminal's text is 144% and re-flows to fewer columns and rows, the right one is unchanged, and the interface zoom and terminal font size preferences are unchanged

#### Scenario: Keyboard
- **WHEN** a terminal has focus and the user presses Ctrl+Shift+= then Ctrl+Shift+0
- **THEN** the whole application zooms in one step and back to 100%, as from any other pane; that terminal's own zoom is unchanged and nothing is sent to the shell

#### Scenario: Bad params
- **WHEN** a zoom command names a pane that is not a terminal, or receives malformed params
- **THEN** it returns an error and no zoom changes
