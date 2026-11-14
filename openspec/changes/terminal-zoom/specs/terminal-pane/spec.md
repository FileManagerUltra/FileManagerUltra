## ADDED Requirements

### Requirement: Per-terminal zoom
Each terminal pane SHALL have its own text zoom from 50% to 400% in ×1.2 steps, applied on top of the terminal font size, independent of other terminals and of the interface zoom, and saved with the pane in its workspace. A zoom handle (zoom out, the current percentage which resets, zoom in) SHALL appear in the terminal's top-right corner while the pointer is over it and whenever its zoom is not 100%. `term.zoom.in`, `term.zoom.out` and `term.zoom.reset` take `{pane?}` (a terminal pane, default the focused one); while a terminal has focus, Ctrl+Shift+=, Ctrl+Shift+- and Ctrl+Shift+0 SHALL run them instead of changing the interface zoom. A stored zoom that is not a finite number in range SHALL be read as bounded (100% when not a number).

#### Scenario: One terminal larger
- **WHEN** two terminals are open and the user clicks + on the left one's handle twice
- **THEN** the left terminal's text is 144% and re-flows to fewer columns and rows, the right one is unchanged, and the interface zoom and terminal font size preferences are unchanged

#### Scenario: Keyboard
- **WHEN** a terminal has focus and the user presses Ctrl+Shift+= then Ctrl+Shift+0
- **THEN** that terminal zooms in one step and back to 100%; nothing is sent to the shell

#### Scenario: Bad params
- **WHEN** a zoom command names a pane that is not a terminal, or receives malformed params
- **THEN** it returns an error and no zoom changes
