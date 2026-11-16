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

### Requirement: Interlaced video is deinterlaced automatically
Video SHALL be deinterlaced without any setting: every picture made from a video (Media pane and Preview playback, picture-subtitle playback, poster frames and folder thumbnails) SHALL pass frames the decoder marks as interlaced (DVDs, TV recordings) through a motion-adaptive deinterlacer before scaling, and SHALL leave progressive frames untouched. It SHALL work with hardware and software decoding alike.

#### Scenario: DVD with fast motion
- **WHEN** an interlaced PAL DVD title with fast horizontal motion plays, or is shown as a poster or thumbnail
- **THEN** no comb lines appear along moving edges

#### Scenario: Progressive video
- **WHEN** a progressive H.264 video plays
- **THEN** its frames are identical to those without the deinterlacer

### Requirement: Video is shown in its display shape
Video SHALL be shown with the shape it is meant to be seen in, not the shape it is stored in: an anamorphic pixel aspect (DVD 16:9) widens the picture, and a stored rotation of 90 or 270 degrees (phone videos filmed upright, from the display matrix or the older `rotate` tag) turns it upright with width and height swapped. This SHALL hold for playback with hardware and software decoding, posters and thumbnails.

#### Scenario: Upright phone video
- **WHEN** a video stored as 1920×1080 with a 90° rotation plays
- **THEN** it is shown upright as 1080×1920, not squeezed into a wide frame

### Requirement: Hardware video decoding
Video SHALL be decoded with the platform's hardware acceleration when available (VA-API on Linux, D3D11/DXVA on Windows, VideoToolbox on macOS) and SHALL fall back to software decoding automatically when hardware decoding is unavailable or fails mid-stream. A preference "Hardware video decoding" (Preferences > Media > Playback) SHALL be on by default and applies to the Media pane and Preview videos; changing it restarts a running decoder. The player menu SHALL show which mode is active ("Decoding: hardware (VA-API)" or "Decoding: software"). When the decode device cannot be opened at all, later videos SHALL skip it until the preference is switched off and on again. Switching to software SHALL restart only the video decoder, so audio continues.

#### Scenario: Unsupported codec
- **WHEN** a video uses a codec the GPU cannot decode, or the machine has no usable decode device
- **THEN** it plays with software decoding, no error or log entry is shown, and the player menu says "Decoding: software"

#### Scenario: Hardware failure
- **WHEN** hardware decoding fails mid-stream
- **THEN** playback restarts from the current position in software mode and a log entry records why (the cause line from ffmpeg, not its closing "Terminating thread" lines)

#### Scenario: Damaged end of a file
- **WHEN** hardware decoding fails within the last 2 seconds of a video, as on a DVD `VTS_xx_1.VOB` part that ends in the middle of a group of pictures
- **THEN** the remaining frames are decoded in software, the player menu says "Decoding: software", and no log entry is written
