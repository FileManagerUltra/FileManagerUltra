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
