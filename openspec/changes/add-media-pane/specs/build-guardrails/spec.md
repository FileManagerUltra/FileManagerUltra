## ADDED Requirements

### Requirement: ffmpeg is discovered, not bundled, and audio output has a documented system dependency
Installers for Linux, Windows and macOS SHALL NOT bundle `ffmpeg` or `ffprobe`. The app SHALL discover them next to the executable, on `PATH` or in package-manager locations, and, where a verified build is pinned (Windows), in the app-managed folder filled by the checksum-verified download specified by the `managed-ffmpeg` change. Audio-only playback of formats decoded in-process SHALL NOT require ffmpeg. The audio output crate's Linux build dependency (ALSA development headers) SHALL be installed by CI and documented in `docs/development.md` as an approved exception to the "no system libraries" rule; the runtime SHALL work without extra packages on desktops that have ALSA, PipeWire or PulseAudio.

#### Scenario: Fresh install without ffmpeg
- **WHEN** an installed app starts on a machine without ffmpeg
- **THEN** in-process audio formats play, and a video reports that ffmpeg is missing with an actionable hint (on Windows, the offer to download the pinned build)

#### Scenario: Installer contents
- **WHEN** the release workflow builds the `.deb`/AppImage, NSIS installer and `.dmg`
- **THEN** none of them contains an `ffmpeg` or `ffprobe` binary
