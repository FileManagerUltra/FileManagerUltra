## ADDED Requirements

### Requirement: Video frames reuse one texture
Each video picture (Media pane, Preview pane) SHALL keep one GPU texture exactly the size of its frames: a frame of the same size SHALL overwrite it in place, and a frame of a new size (another video, a resized pane) SHALL re-specify it once. With "Reuse video texture" on (the default) the texture SHALL survive up to 5 seconds of frames without a picture (seek, decoder restart) and be released after that or when its pane closes; with it off it SHALL be released at the first frame that does not draw it.

#### Scenario: Resizing while playing
- **WHEN** a 1080p video plays and the pane is resized so the decode width changes several times
- **THEN** the texture is re-specified once per decode-width change, never per frame, and memory does not grow with each resize

#### Scenario: Seeking
- **WHEN** the user seeks repeatedly in a playing video
- **THEN** the same texture is used across the decoder restarts
