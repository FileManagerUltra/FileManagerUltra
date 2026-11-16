## ADDED Requirements

### Requirement: Pane zoom
Every pane header SHALL have a zoom button (a magnifying glass with a plus) just left of the pane type selector. Clicking it, or Enter/Space while it has keyboard focus, SHALL open the pane's zoom controls stacked vertically: zoom in (+), the current percentage (which resets to 100%) and zoom out (−). The popup SHALL close on a click outside it or on Escape, and Escape SHALL return keyboard focus to the zoom button. The controls SHALL meet WCAG 2.2 AA: while the pane's zoom is not 100% the button SHALL show the percentage as text left of the icon, plus an outline, so the state is not shown by colour alone (1.4.1); the button SHALL always reserve the width of the widest percentage, so showing or changing the percentage moves neither the button, the header beside it nor the open popup (no layout shift); the button and each popup button SHALL be at least 24×24 points (2.5.8); icon and text SHALL have at least 4.5:1 contrast and the outline and focus ring at least 3:1 against the header and its hover fill, in both themes and for any accent colour (1.4.3, 1.4.11); the button SHALL be named "Zoom" (or "Zoom, N%" while zoomed) and report whether its popup is open (expanded), and the popup buttons SHALL be named "Zoom … in", "Reset … zoom (N%)" and "Zoom … out" (1.1.1, 2.5.3, 4.1.2); Tab SHALL move from the button into the popup (2.1.1, 2.4.3). The controls SHALL dispatch `pane.scale.in`, `pane.scale.out` and `pane.scale.reset` (`{pane?}`, default the focused pane), which change the zoom of that pane only: a Terminal pane's terminal zoom, a Preview pane's preview zoom, and for Files, Network, SFTP, Trash and Media panes a pane zoom from 50% to 400% in ×1.2 steps that scales the pane's text, row height and spacing, and in a Media pane also its player controls (transport and toolbar buttons with their icons, seek and volume bars, clock, title, cover art, playlist numbers, marks and indents) (not its header), is saved with the pane in its workspace, and leaves the interface zoom and other panes unchanged. A stored pane zoom that is not a finite number in range SHALL be read as bounded (100% when not a number). These commands have no default shortcut. Ctrl+= / - / 0 still zoom a focused Preview; Ctrl+Shift+= / - / 0 SHALL be the interface zoom in every pane, a terminal or Preview included.

#### Scenario: Controls stay out of the way
- **WHEN** a pane is shown
- **THEN** its header shows only the zoom button (and the percentage while zoomed), not the zoom controls, until the button is clicked or activated from the keyboard

#### Scenario: No layout shift
- **WHEN** the popup is open and the user clicks + (100% → 120%) and then − twice
- **THEN** the button, its icon and the popup stay exactly where they were; only the percentage text appears and changes

#### Scenario: Keyboard only
- **WHEN** the zoom button has focus and the user presses Enter, Tab, Space and then Escape
- **THEN** the popup opens, focus moves to zoom in, the pane zooms in one step, the popup closes, and focus is back on the button, which now reads "Zoom, 120%" and reports collapsed

#### Scenario: One Files pane larger
- **WHEN** two Files panes are open and the user opens the left one's zoom and clicks + twice
- **THEN** the left pane's rows and text are drawn at 144%, its zoom button reads 144% with an outline, and the right pane, both headers and the interface zoom are unchanged

#### Scenario: Media player controls zoom too
- **WHEN** a Media pane is zoomed to 200%
- **THEN** its previous, play, stop and next buttons and their icons are twice as large, the seek bar twice as tall, and the playlist numbers and names stay aligned without overlapping

#### Scenario: Terminal and Preview keep their own zoom
- **WHEN** the header zoom of a Terminal or Preview pane is used
- **THEN** it changes that terminal's or preview's existing zoom (the same value its keyboard shortcuts change), not a separate pane zoom

#### Scenario: Bad params
- **WHEN** `pane.scale.*` names an unknown pane or receives malformed params
- **THEN** it returns an error and no zoom changes
