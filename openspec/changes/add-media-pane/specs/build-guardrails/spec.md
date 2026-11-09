## ADDED Requirements

### Requirement: ffmpeg is bundled and audio output has a documented system dependency
Installers for Linux, Windows and macOS SHALL bundle an `ffmpeg` and `ffprobe` sidecar that the app discovers before `PATH`, built as LGPL (no GPL or non-free components), with the licence text and a source offer shipped in the package. Release builds SHALL fetch pinned builds and verify their checksums. The audio output crate's Linux build dependency (ALSA development headers) SHALL be installed by CI and documented in `docs/development.md` as the third approved exception to the "no system libraries" rule; the runtime SHALL work without extra packages on desktops that have ALSA, PipeWire or PulseAudio.

#### Scenario: Fresh install
- **WHEN** an installed app starts on a machine without ffmpeg on `PATH`
- **THEN** the bundled sidecar is found and video plays

#### Scenario: Checksum mismatch
- **WHEN** a downloaded ffmpeg build does not match its pinned checksum
- **THEN** the release build fails
