## ADDED Requirements

### Requirement: Interface zoom
The View menu SHALL offer Zoom In, Zoom Out and Reset Zoom (`view.zoom.in|out|reset`, Ctrl+Shift +/-/0). The factor SHALL stay within 0.5-3.0 and be stored in preferences and restored at start-up.

#### Scenario: Zoom persists
- **WHEN** the user zooms in and restarts the app
- **THEN** the interface opens at the same zoom
