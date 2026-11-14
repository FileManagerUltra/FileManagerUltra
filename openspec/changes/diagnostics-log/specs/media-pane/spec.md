## ADDED Requirements

### Requirement: Video frames reuse one texture
With "Reuse video texture" on (the default), each video picture (Media pane, Preview pane) SHALL keep one GPU texture whose size only grows, rounded up to 256-pixel steps, and SHALL write every frame into it in place, drawing only the part the frame covers. The texture SHALL survive up to 5 seconds of frames without a picture (seek, decoder restart) and be released after that or when its pane closes. With the switch off, the previous behaviour SHALL apply (a same-size frame updates in place, a new size re-creates the texture, a frame without a picture releases it).

#### Scenario: Resizing while playing
- **WHEN** a 1080p video plays and the pane is resized so the decode width changes several times
- **THEN** the texture is re-created only when a frame is larger than its current storage, and memory does not grow with each resize

#### Scenario: Seeking
- **WHEN** the user seeks repeatedly in a playing video
- **THEN** the same texture is used across the decoder restarts
