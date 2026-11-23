## MODIFIED Requirements

### Requirement: ffmpeg is discovered, not bundled, and audio output has a documented system dependency
Installers for Linux, Windows and macOS SHALL NOT bundle `ffmpeg` or `ffprobe`. The app SHALL discover them next to the executable, on `PATH` or in package-manager locations, and, where a verified build is pinned (Windows), in the app-managed folder filled by the checksum-verified download specified by the `managed-ffmpeg` change. The App Store edition SHALL NOT discover or run an external `ffmpeg`, because the sandbox cannot run it; its ffmpeg-dependent features are absent as specified by `app-store-edition`. Audio-only playback of formats decoded in-process SHALL NOT require ffmpeg. The audio output crate's Linux build dependency (ALSA development headers) SHALL be installed by CI and documented in `docs/development.md` as an approved exception to the "no system libraries" rule; the runtime SHALL work without extra packages on desktops that have ALSA, PipeWire or PulseAudio.

#### Scenario: Fresh install without ffmpeg
- **WHEN** an installed app starts on a machine without ffmpeg
- **THEN** in-process audio formats play, and a video reports that ffmpeg is missing with an actionable hint (on Windows, the offer to download the pinned build)

#### Scenario: Installer contents
- **WHEN** the release workflow builds the `.deb`/AppImage, NSIS installer, `.dmg` and App Store `.pkg`
- **THEN** none of them contains an `ffmpeg` or `ffprobe` binary

#### Scenario: App Store edition with Homebrew ffmpeg
- **WHEN** the App Store edition runs on a Mac with ffmpeg on `PATH`
- **THEN** it does not look for or start ffmpeg

#### Scenario: Linux runtime libraries
- **WHEN** the `.deb` is built
- **THEN** its `Depends` lists the binary's runtime libraries (`libc6 (>= 2.35)`, `libgcc-s1`, `libasound2t64 | libasound2`), so apt installs ALSA where it is missing; the AppImage leaves `libasound.so.2` to the host, as the AppImage excludelist does
