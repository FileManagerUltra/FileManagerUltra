## ADDED Requirements

### Requirement: ffmpeg can be installed on demand
Where a verified build is pinned (Windows), Preferences > Media SHALL show where ffmpeg was found (system or app-managed), and offer Download, Update (when a newer pin ships in this app version) and Reinstall. The system install is always preferred; the app-managed copy is searched last. Downloading runs on a background thread with progress and never blocks the UI.

#### Scenario: System ffmpeg present
- **WHEN** ffmpeg and ffprobe are found next to the executable or on `PATH`
- **THEN** they are used, the section reports the location, and no download is started

#### Scenario: Missing ffmpeg
- **WHEN** no ffmpeg is found and a pin exists for the platform
- **THEN** the hint on a video card or error names Preferences > Media, and Download starts the verified installation; on success playback works without restarting

#### Scenario: Newer pin
- **WHEN** the installed manifest version differs from the app's pin
- **THEN** the section offers Update; the old copy keeps working until the new one is verified and renamed into place

#### Scenario: Failed install
- **WHEN** the download fails, is rejected, or the files cannot be replaced
- **THEN** the section shows the reason and the previous install, if any, stays usable
