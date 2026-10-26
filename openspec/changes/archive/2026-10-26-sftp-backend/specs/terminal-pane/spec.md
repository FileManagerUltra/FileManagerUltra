## ADDED Requirements

### Requirement: Close on shell exit
A terminal pane whose shell exits (exit, Ctrl-D) SHALL close. The last remaining pane stays and shows the restart banner.

#### Scenario: Exit closes the pane
- **WHEN** the shell in a terminal pane exits and other panes exist
- **THEN** the pane is closed
