## ADDED Requirements

### Requirement: Playing time in the status bar
While the status bar shows the media controls, it SHALL also show the playing position as `elapsed / total` in the player's clock format (`m:ss`, or `h:mm:ss` from an hour), or only `elapsed` when the length is unknown, in fixed-width digits with theme colours. While playing, the readout SHALL be redrawn when the shown second changes and not more often for its own sake; while paused or minimized it SHALL NOT cause redraws.

#### Scenario: Media pane hidden
- **WHEN** a song plays, the Media pane is closed and the user works in a Files pane
- **THEN** the status bar shows e.g. `1:23 / 4:56` next to the controls and it advances once a second

#### Scenario: Paused
- **WHEN** playback is paused at 2:05 of a 1:02:00 audiobook
- **THEN** the readout shows `2:05 / 1:02:00` and the window draws no frames for it
