## ADDED Requirements

### Requirement: Window fullscreen
`window.fullscreen` SHALL toggle native fullscreen of the main window (menu View, shortcut F11 where the platform has no reserved one). Entering or leaving fullscreen SHALL NOT replace the saved normal geometry, SHALL preserve the layout, focus and zoom, and a normal quit while fullscreen SHALL restore the last normal size and position at next launch. Fullscreen state SHALL NOT be persisted across launches.

#### Scenario: Toggle
- **WHEN** `window.fullscreen` runs twice
- **THEN** the window enters and leaves fullscreen with the same layout, focus and normal geometry

#### Scenario: Quit in fullscreen
- **WHEN** the app is closed while fullscreen
- **THEN** the next launch opens in the last normal window geometry, not fullscreen
