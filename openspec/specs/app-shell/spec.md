# app-shell

## Purpose

The application window: menus, status bar, clean quit, cross-platform build. Owned by `apps/fm-app` (L5) on top of the engine.
## Requirements
### Requirement: The application opens a window
The application SHALL open a native window when launched on a supported platform (Linux, macOS, Windows), showing the product title, a menu bar, a status bar and a placeholder central area.

#### Scenario: Launch on a desktop session
- **WHEN** the user runs the application binary on a machine with a display
- **THEN** a window appears whose title is the product name from `fm-brand`
- **AND** the central area shows a placeholder ("no panes yet") instead of an empty region
- **AND** no panic, blank-white window or error dialog occurs on the healthy path

#### Scenario: Headless environment
- **WHEN** the application is launched without a usable display
- **THEN** it exits with a non-zero status and a clear error message on stderr, without panicking or leaving a zombie process

### Requirement: The menu bar exposes registered commands
The menu bar SHALL render its items from the command registry, and selecting an item SHALL dispatch that command by id.

#### Scenario: Seeded menus exist
- **WHEN** the window has opened
- **THEN** the menu bar contains the File, View and Help menus
- **AND** each item maps to a command id that exists in the registry (verified by a test over the menu catalogue)

#### Scenario: Disabled commands are visibly disabled
- **WHEN** a command's `enabled` predicate returns false for the current session
- **THEN** its menu item renders greyed out and activating it does nothing

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

### Requirement: Consistent draggable dialogs
All floating dialogs SHALL follow `docs/style-guide.md`. They include About, Preferences,
file-operation prompts, conflict and unsaved-workspace confirmations, connection and
host-key prompts, key-file picker, Properties, Find Files, Open With, log time range and Jobs: scoped proportional typography, neutral headings, flat
sections, aligned controls, semantic theme colors and shared window chrome.
Every dialog SHALL be draggable by its title bar and SHALL have a title-bar close
button. Initial placement SHALL be centered except Jobs, which starts bottom-right.
Dragged positions SHALL persist across frames and viewport resizing, subject to
keeping the window on screen.

#### Scenario: Close without approving
- **WHEN** the title-bar close button is clicked
- **THEN** the dialog follows its existing Cancel or dismiss behavior
- **AND** no destructive operation, permission edit, host-key trust or pending form submission is approved
- **AND** closing Preferences preserves changes already applied
- **AND** closing a child key-file picker leaves the connection form open

#### Scenario: Hide and reopen Jobs
- **WHEN** Jobs is closed using its title-bar button
- **THEN** the panel stays hidden without cancelling operations or dismissing updates
- **AND** clicking the status-bar indicator reopens it

### Requirement: Quit exits cleanly
The application SHALL close without panic when the user quits, and SHALL persist preferences before exiting.

#### Scenario: Restore window geometry
- **WHEN** the user moves or resizes the main window and closes the application normally
- **THEN** the next launch restores its normal size and desktop position, along with maximized state
- **AND** application zoom does not change the stored native dimensions
- **AND** maximizing, fullscreen or minimizing does not replace the last normal geometry
- **AND** geometry is saved about a second after it stops changing, so a killed process or ended session still restores it
- **AND** a window manager that places the new window elsewhere is overridden with the saved position
- **AND** invalid geometry falls back to default dimensions and positions on a disconnected monitor are left to the window manager
- **AND** on native Wayland, size is restored but window positioning remains compositor-controlled

#### Scenario: Quit from the menu or shortcut
- **WHEN** the user triggers `app.quit`
- **THEN** the window closes and the process exits with status 0
- **AND** any changed preferences (e.g. theme) are written to `prefs.toml`

#### Scenario: Window close button
- **WHEN** the user closes the window through the window manager
- **THEN** the same clean shutdown path runs (preferences saved, no zombie worker threads)

### Requirement: The workspace builds on all three platforms
The workspace SHALL compile for x86_64 Linux (verified in every session) and SHALL keep Windows and macOS targets buildable — no platform-gated code without a `cfg` for the other two.

#### Scenario: Linux build gate
- **WHEN** `cargo build --workspace` runs on Linux
- **THEN** it succeeds with no warnings under `cargo clippy --workspace --all-targets -- -D warnings`

#### Scenario: Platform-specific code is gated
- **WHEN** any source file contains `#[cfg(...)]` platform code
- **THEN** the other platforms still compile (checked by the CI task on the platforms available, and by review otherwise)

### Requirement: Window fullscreen
`window.fullscreen` SHALL toggle native fullscreen of the main window (menu View, shortcut F11 where the platform has no reserved one). Entering or leaving fullscreen SHALL NOT replace the saved normal geometry, SHALL preserve the layout, focus and zoom, and a normal quit while fullscreen SHALL restore the last normal size and position at next launch. Fullscreen state SHALL NOT be persisted across launches.

#### Scenario: Toggle
- **WHEN** `window.fullscreen` runs twice
- **THEN** the window enters and leaves fullscreen with the same layout, focus and normal geometry

#### Scenario: Quit in fullscreen
- **WHEN** the app is closed while fullscreen
- **THEN** the next launch opens in the last normal window geometry, not fullscreen

### Requirement: Drawing does not grow memory
Drawing frames SHALL NOT make the application's memory grow. On Windows the window SHALL be drawn with Direct3D 12 by default rather than OpenGL, because OpenGL drivers there (NVIDIA with "Threaded optimization") keep memory for every drawn frame.

#### Scenario: Window visible on Windows with an NVIDIA GPU
- **WHEN** the application is visible for ten minutes with video playing or the pointer moving over it, with the driver's default settings
- **THEN** its commit size stays flat apart from what open files and caches need, as it does while minimized

### Requirement: Renderer choice on Windows
On Windows, Preferences › Advanced › Troubleshooting SHALL offer *Renderer*: Direct3D (default) or OpenGL, applied at the next start, and SHALL show which renderer and GPU are in use. If Direct3D cannot start, the application SHALL open with OpenGL instead and say so in a notice. Other platforms draw with OpenGL and do not show the choice.

#### Scenario: Dragging the window with Direct3D
- **WHEN** the user drags the window by its title bar on Windows with Direct3D
- **THEN** the window follows the pointer as smoothly as with OpenGL, because moving the window does not redraw its contents

#### Scenario: Choosing OpenGL
- **WHEN** the user selects OpenGL and restarts the application
- **THEN** the window is drawn with OpenGL and the row shows "In use: OpenGL"

#### Scenario: Direct3D unavailable
- **WHEN** Direct3D 12 cannot start on the machine, or only offers a software adapter
- **THEN** the window opens with OpenGL and a notice explains that Direct3D could not start
- **AND** the notice is not repeated at later starts while the situation stays the same, though Direct3D is tried again each time

#### Scenario: No usable graphics card
- **WHEN** neither Direct3D 12 nor OpenGL can use a graphics card (for example a virtual machine without 3D acceleration)
- **THEN** the window opens drawn by Direct3D's software rasterizer and a notice says drawing is slower

### Requirement: One prompt before anything is lost
Quitting, closing a pane and closing a workspace tab SHALL ask at most one question. When the action would lose unsaved workspace changes, filled-in PDF fields or signatures, or staged archive changes, or (on quit) stop work still running, a single prompt SHALL list all of them, grouped by kind, with Save (save everything listed, then go on), Don't save (go on without saving) and Cancel. When only running work is affected the prompt SHALL offer Keep running and Quit anyway. Save SHALL go on only after every save it started has finished; a failed or skipped save SHALL cancel the quit or close, keeping everything. A remembered "Don't ask again" answer for workspaces SHALL still apply without listing them.

#### Scenario: Workspace, PDF and archive changes on quit
- **WHEN** a workspace, a filled-in PDF and `site.zip` all have unsaved changes and the user quits
- **THEN** one prompt lists the workspace, the PDF and `site.zip`, and Save saves all three before the app quits, without a second prompt

#### Scenario: Closing one of two panes on an archive
- **WHEN** two panes show `site.zip` with staged changes and the user closes one of them
- **THEN** nothing is asked; closing the second one asks about `site.zip`

#### Scenario: Only running work
- **WHEN** a copy is running and nothing is unsaved, and the user quits
- **THEN** one prompt lists the copy with Keep running and Quit anyway

