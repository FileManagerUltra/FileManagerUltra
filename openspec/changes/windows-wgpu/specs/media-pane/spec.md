## ADDED Requirements

### Requirement: Video redraws at its frame rate
While a video plays, the window SHALL be redrawn when the decoder has a new frame rather than on a fixed fast timer, and only while a playing player (Media pane or Preview) is on screen. Other redraws while playing (clock, seek bar) SHALL be at most four per second.

#### Scenario: 25 fps video
- **WHEN** a 25 fps video plays in a visible Media pane without other activity
- **THEN** the window draws about 25 frames per second, not 60

#### Scenario: Player hidden
- **WHEN** a video keeps playing while its pane is hidden or the window is minimized
- **THEN** new video frames do not cause redraws
