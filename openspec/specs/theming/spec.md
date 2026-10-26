# theming

## Purpose

Dark/light/system theming from semantic tokens, switched at runtime through commands and persisted in prefs.
## Requirements
### Requirement: Theme follows the system by default
On first run the application SHALL render in the theme the operating system reports (dark or light), with no user action required.

#### Scenario: System reports dark
- **WHEN** the OS theme preference is dark and no user preference has been saved
- **THEN** the window renders with dark visuals (light text on dark surfaces)

#### Scenario: System reports light
- **WHEN** the OS theme preference is light and no user preference has been saved
- **THEN** the window renders with light visuals

### Requirement: The theme is switchable at runtime
The user SHALL be able to switch between dark, light and follow-the-system themes while the application is running, through commands available in the View menu.

#### Scenario: Toggle with the keyboard
- **WHEN** the user presses the theme toggle shortcut (`Ctrl+Shift+T`)
- **THEN** the theme switches between dark and light immediately, without restarting
- **AND** the window re-renders in the new theme on the next frame

#### Scenario: Choose follow-the-system
- **WHEN** the user picks View ▸ Theme ▸ System
- **THEN** the rendered theme matches the OS preference again, including when the OS theme changes later

### Requirement: The theme choice persists across restarts
A theme chosen by the user SHALL be written to `prefs.toml` in the platform config directory and restored on the next launch.

#### Scenario: Persist and restore
- **WHEN** the user switches to dark mode and quits
- **THEN** `prefs.toml` records the dark preference
- **AND** the next launch renders dark even if the OS reports light

#### Scenario: Corrupt or missing prefs file
- **WHEN** `prefs.toml` is missing, unreadable or contains invalid data
- **THEN** the application starts with the system default theme and reports no error to the user (the bad file is ignored, never a panic)

### Requirement: Colours come only from semantic tokens
Every widget SHALL derive its colours from the `fm-ui` theme token set; colour literals SHALL appear only in the theme module, and dark and light token sets SHALL stay in lockstep.

#### Scenario: Token coverage
- **WHEN** a new token field is added to `Tokens`
- **THEN** both `Tokens::dark()` and `Tokens::light()` must define it or the code fails to compile (exhaustive destructuring)

#### Scenario: No stray colour literals
- **WHEN** the brand-check-style source scan runs over `crates/ui` and `apps/fm-app`
- **THEN** a `Color32::` literal outside the theme module fails the check

#### Scenario: Both themes are verified
- **WHEN** UI snapshot tests run
- **THEN** each captured state exists in both dark and light variants

### Requirement: Interface zoom
The View menu SHALL offer Zoom In, Zoom Out and Reset Zoom (`view.zoom.in|out|reset`, Ctrl+Shift +/-/0). The factor SHALL stay within 0.5-3.0 and be stored in preferences and restored at start-up.

#### Scenario: Zoom persists
- **WHEN** the user zooms in and restarts the app
- **THEN** the interface opens at the same zoom

