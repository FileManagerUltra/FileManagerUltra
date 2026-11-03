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

Preferences SHALL use proportional typography, left-aligned topic labels and neutral page headings without changing typography elsewhere in the application. Settings groups SHALL use flat sections with thin dividers, consistent spacing and aligned controls rather than nested cards. Trusted hosts SHALL appear in a bordered list with readable rows and aligned Forget actions. Mutually exclusive theme and shell choices SHALL use radio controls.

The dialog SHALL initially select General, remember the selected topic while the application runs, and retain a consistent size when switching topics. Settings and navigation SHALL scroll when needed so the footer and Close remain accessible in small windows. Both dark and light themes SHALL use semantic theme tokens.

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
