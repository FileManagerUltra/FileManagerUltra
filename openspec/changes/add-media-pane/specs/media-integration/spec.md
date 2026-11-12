## ADDED Requirements

### Requirement: System media controls
Playback SHALL integrate with the operating system's media controls: MPRIS on Linux, System Media Transport Controls on Windows and the Now Playing / remote command centre on macOS. They SHALL show title, artist, album, cover art and position, and SHALL let hardware media keys and the system overlay play, pause, stop, next, previous and seek. Integration SHALL be registered when playback starts, removed when it ends, and SHALL be best effort: failure to register SHALL produce a log entry and never prevent playback. Only commands from the controls SHALL dispatch (`media.*` ids); they SHALL ignore requests to open arbitrary files or URLs.

#### Scenario: Hardware play/pause key
- **WHEN** the media key is pressed while the app is in the background and music plays
- **THEN** playback pauses

#### Scenario: Remote open request
- **WHEN** another program asks the MPRIS interface to open a URI
- **THEN** the request is ignored

#### Scenario: Play key with files selected
- **WHEN** Media is paused, files are selected in a Files pane and the Play media key is pressed
- **THEN** the paused item resumes; the selection is not played

#### Scenario: Cover art for the overlay
- **WHEN** the current item has embedded cover art
- **THEN** the controls receive a `file://` URL to the app's own re-encoded PNG (at most 512 px, owner-only file in an owner-only per-process folder), never the embedded bytes; it is replaced on the next item and removed on quit

#### Scenario: No session bus
- **WHEN** the controls cannot register (no D-Bus session, name taken, OS refusal)
- **THEN** one log line is written, playback is unaffected, and registration is tried again only after playback has ended and started again

### Requirement: Hardware video decoding
Video SHALL be decoded with the platform's hardware acceleration when available (VA-API on Linux, D3D11/DXVA on Windows, VideoToolbox on macOS) and SHALL fall back to software decoding automatically when hardware decoding is unavailable or fails mid-stream. A preference "hardware decoding" SHALL be on by default and the pane SHALL show which mode is active.

#### Scenario: Unsupported codec
- **WHEN** a video uses a codec the GPU cannot decode
- **THEN** it plays with software decoding and no error is shown

#### Scenario: Hardware failure
- **WHEN** hardware decoding fails mid-stream
- **THEN** playback restarts from the current position in software mode and a log entry records why
