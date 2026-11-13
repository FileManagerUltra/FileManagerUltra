## MODIFIED Requirements

### Requirement: Topic-based preferences
Preferences SHALL present a simplified two-pane dialog with a flat topic list on the left and only the selected topic's settings on the right. Topics SHALL be General, Appearance, Files, Copy and move, Preview, Terminal, Connections, Workspaces, Keyboard shortcuts and Advanced. Appearance SHALL offer theme, accent colour (presets or `#rrggbb`), density and text size; Files SHALL offer hidden-files default, single-click open, size units, date format and confirmation prompts; Terminal SHALL offer shell and font size; Advanced SHALL offer the thumbnail cache location (shown, changeable to a parent folder that receives a `thumbs` subfolder, resettable), a Clear thumbnail cache button, the cache size limits and the preview time limit. Every such setting SHALL change through the validated `prefs.set` command (bad values return an error and change nothing) and be clamped when read from a hand-edited prefs file. Connections SHALL contain SMB encryption, remote timing and trusted-host management.

Preferences SHALL use proportional typography, left-aligned topic labels and neutral page headings without changing typography elsewhere in the application. Settings groups SHALL use flat sections with thin dividers, consistent spacing and aligned controls rather than nested cards. Trusted hosts SHALL appear in a bordered list with readable rows and aligned Forget actions. Mutually exclusive choices (theme, density, size units, date format, shell, media opening, accent and subtitle colours, subtitle font and edge) SHALL use choice rows that behave as radio groups; there SHALL be no dropdowns in Preferences.

The dialog SHALL initially select General, remember the selected topic while the application runs, and retain a consistent size when switching topics. Settings and navigation SHALL scroll when needed so the footer and Close remain accessible in small windows. Both dark and light themes SHALL use semantic theme tokens.

Preferences SHALL be fully usable with the keyboard. Tab and Shift+Tab SHALL cycle within the dialog: the topic list, then the current topic's controls in reading order, then Close, and back to the topic list; focus SHALL NOT move to panes behind the dialog by Tab. The mouse SHALL still reach those panes. The topic list and each choice row SHALL be a single Tab stop (the selected item); Up/Down/Home/End select a topic and arrow keys select and apply a choice. Ctrl+Tab, Ctrl+Shift+Tab, Ctrl+PageDown and Ctrl+PageUp SHALL switch topics from anywhere in the dialog, and opening Preferences SHALL put the keyboard on the topic list. While the keyboard is in use, the focused control SHALL show a clearly visible focus ring in both themes and SHALL be scrolled into view. Escape SHALL first close an open popup, then leave a text field, and only then close the dialog. In File types, Tab and Shift+Tab SHALL move between the types list, the patterns list and the following controls without trapping focus, and pattern actions SHALL have keys (Insert adds, Delete removes, F2 or Enter edits).

#### Scenario: Walk the dialog with Tab
- **WHEN** Preferences is open on General and the user presses Tab repeatedly
- **THEN** focus goes from the General topic through each General control in order to Close and then back to General, never to a pane behind the dialog
- **AND** Shift+Tab walks the same stops in reverse

#### Scenario: Choose with arrow keys
- **WHEN** the Theme row has focus on System and the user presses Right
- **THEN** Dark is selected and applied, the row keeps the focus, and the next Tab leaves the row

#### Scenario: Switch topics from the keyboard
- **WHEN** the topic list has focus and the user presses Down, or presses Ctrl+Tab anywhere in the dialog
- **THEN** the next topic is shown

#### Scenario: Browse keyboard shortcuts
- **WHEN** Keyboard shortcuts is selected
- **THEN** a read-only reference lists all registered command shortcuts, including currently disabled commands, plus binding aliases and pane-local navigation keys
- **AND** shortcuts are grouped by application, workspace, pane layout, file navigation, selection/clipboard, file operations, preview, logs/diffs, terminal and text/dialog context
- **AND** modifier names follow the platform (Cmd on macOS, Ctrl elsewhere), without relabeling shell Control keys
- **AND** a search field filters actions, key combinations, groups and context descriptions
- **AND** shared combinations explain focus-dependent behavior, including Preview Find, diff F7 navigation, terminal input and workspace switching
- **AND** browsing or searching changes neither preferences nor keyboard bindings

#### Scenario: Change a setting in a topic
- **WHEN** a topic is selected
- **THEN** its settings appear without settings from other topics
- **AND** edits dispatch the existing preference commands immediately, without an extra Apply step
- **AND** switching topics preserves edits and does not change settings by itself

#### Scenario: Close and reopen preferences
- **WHEN** Close or Escape dismisses Preferences and it is reopened
- **THEN** the last topic is selected and previous edits remain applied
- **AND** the existing preference persistence policy is unchanged
