## ADDED Requirements

### Requirement: Opening choices are shown as a table
Preferences → Media → Opening files SHALL show one row per kind (Audio, Video), each with example extensions in muted text, and a segmented choice in one aligned column: *Media pane* (the *Play in Media pane* value) first, then *Default app* (the *Open in default app* value). Each segment SHALL have an accessible name that includes the row ("Video: Media pane"), the choice SHALL be one Tab stop moved with the arrow keys, and Preferences → Editor → Opening files SHALL use the same layout.

#### Scenario: Choosing per kind
- **WHEN** the user clicks "Video: Media pane"
- **THEN** video files play in the Media pane and the audio choice is unchanged
