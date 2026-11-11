## ADDED Requirements

### Requirement: Media files, playlists and decoders are untrusted and bounded
Media files, tags, cover art, subtitles and playlists SHALL be treated as hostile input. Decoders other than `symphonia` for audio SHALL run as subprocesses with the discovered tool path, no shell, an argument list built from validated paths, a wall-clock timeout for probing, and killed on stop, pane close or app quit. Playlists SHALL be capped (size, entry count, line length, nesting), entries SHALL be parsed as paths or `VfsUri`s and never executed, and `http`, `file` or other URL schemes in playlist entries SHALL be refused. Embedded cover art and tag text SHALL be size-capped before decoding. The audio buffer and frame queue SHALL be bounded. Persisted queue state SHALL contain paths only and be owner-only (`0600`/`0700`); spooled remote files follow the temporary-file rules. Credentials SHALL never appear in playlists, queue state, notices or logs.

#### Scenario: Hostile subtitle file
- **WHEN** a subtitle file is binary, larger than 8 MiB, has more than 100 000 cues, absurd timestamps or huge lines
- **THEN** it is refused or truncated to the caps with an error notice, nothing panics, and subtitle extraction runs as a capped, cancellable `ffmpeg` subprocess restricted to the `file` protocol

#### Scenario: Hostile disc image
- **WHEN** a `.iso` has a corrupt or oversized file table, records pointing past its end, or no DVD-Video folder
- **THEN** it is refused with an error and nothing panics; directory reads are capped at 1 MiB, ranges are checked against the image length, and ffmpeg reads the title only through `subfile` over the `file` protocol

#### Scenario: Oversized playlist
- **WHEN** a playlist larger than the cap or with too many entries is loaded
- **THEN** it is rejected with an actionable error and nothing is enqueued

#### Scenario: URL entry
- **WHEN** a playlist contains `http://` or `file://` entries
- **THEN** those entries are skipped with a notice and never fetched or launched

#### Scenario: Hung decoder
- **WHEN** a decoder subprocess stops responding
- **THEN** it is killed after its timeout, an error notice is shown and the next item plays

#### Scenario: Corrupt media
- **WHEN** a decoder fails or panics on a corrupt file
- **THEN** the app keeps running and playback advances
