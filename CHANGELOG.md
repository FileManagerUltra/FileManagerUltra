# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Play local audio in a dedicated Media pane with a queue, seeking, volume, mute, shuffle and repeat. Audio can keep playing after the pane closes, with a Stop control in the status bar; starting Preview playback pauses it.
- The Media queue and playback settings now return paused after restarting; missing files are skipped with a notice.
- Import local M3U, M3U8 and PLS playlists, or save the queue as a new M3U8 file. Unsafe links, missing files and unsupported entries are skipped with notices.
- Drop local audio files or folders onto Media to queue them without copying or moving files. Folders play in natural filename order; idle drops start playback.
- Files and Preview now offer Play, Play next, Add to queue and Play folder for local audio. Add appends; Play next inserts after the current track; Play inserts there and starts immediately without discarding the playlist. Playing from Preview continues at its current position.
- Play local video in Media with sound, seeking and frame-by-frame stepping while paused. Files, folders, playlists and Preview handoff also accept video; closing the Media pane stops video without clearing the queue.
- Adjust local video playback speed from 0.5× to 3× while preserving pitch.
- Choose the audio track and toggle subtitles for videos with several streams (subtitle rendering is not yet available).
- Fullscreen video in Media (F, double-click or the Fullscreen button; Esc leaves) with controls and cursor that hide after 3 seconds, plus F11 for window fullscreen.
- YouTube-style keyboard controls in Media (Space/K, J/L, arrows, 0–9, Home/End, M, N/P, comma/period, < and >); a Keys menu lists them, and Preview audio and video get the basic transport keys.
- Play audio and video from SFTP and SMB: the file is downloaded to a private temporary folder first, with progress shown, and removed afterwards.
- Gapless playback: consecutive local tracks join without a pause, honouring encoder delay and padding (can be switched off in the Media pane).
- A ten-band equalizer for Media audio and video with built-in and saved presets, an automatic preamp to avoid clipping, and settings that persist.
- A-B loop to repeat part of a track, and a sleep timer (minutes or end of the current item) that fades out before pausing.
- Play individual audio files in Preview, with Play/Pause, seeking, volume, mute, tags and embedded cover art. Audio autoplay is optional and off by default.
- Preview videos now play with sound, volume and mute controls, and synchronized picture and audio. Video autoplay can be disabled in Preferences; a missing audio device leaves the video playing silently with a hint.

## [0.1.3] - 2026-11-09

### Changed

- Sorting by name (and type) is now natural: `f2` comes before `f10`, numbers inside names compare by value.

### Fixed

- Ctrl+click (Cmd+click on macOS) now opens links in the terminal pane even when the running program, such as Copilot CLI, captures the mouse. Opening a link shows a short "Opening … in your default browser" message.
- Copying from a program in the Terminal pane that uses its own selection, such as Copilot CLI, now reaches the system clipboard (only from the focused pane).
- Typing a Windows-style network address such as `\\192.168.0.50` or `\\nas\media\Movies` in the path bar now connects to that server over SMB (on Windows this applies in network panes; file panes keep using Windows' own network access).
- Shift-click now adds to the selection: clicking below it extends from its first file, clicking above it extends from its last file, instead of replacing it.
- The window no longer freezes while a sleeping hard disk spins up when a folder on it is open (at start-up or later): the window stays movable and responsive while the folder loads.

## [0.1.2] - 2026-11-08

### Added

- Drag files out of a pane and drop them into Explorer, a browser upload field or a mail/chat window on Windows. Press Escape to cancel.

### Changed

- Errors and warnings no longer appear in the status bar. They open the jobs panel for five seconds (it stays open while the pointer is over it), and a new icon at the bottom right of the status bar reopens it to show recent messages.
- Scrolling with the mouse wheel is faster in the preview panel (thumbnails and documents).

### Fixed

- On Windows, a folder that cannot be watched (for example an inaccessible profile folder) no longer shows a "Live refresh is unavailable" warning; it is simply refreshed periodically.

## [0.1.1] - 2026-11-06

Beta release.

### Added

- Right-click an image (in the file list, a thumbnail or the image preview) and choose **Copy Image** to put the original, full-size picture on the clipboard, ready to paste into a browser or editor. Also in the Edit menu.
- Drag files out of a pane and drop them into a browser (upload fields), chat or mail window on Linux. Works with browsers running natively on Wayland as well as X11, and the app now runs natively on Wayland sessions (set `FM_X11=1` to force XWayland); the window position is no longer restored there. macOS and Windows are not supported yet.
- The Markdown preview shows images (PNG, JPEG, GIF, WebP, BMP, ICO, SVG), including inside tables. Relative images next to the document load automatically; remote (http/https) images load only when you enable "Load remote images in Markdown" in Preferences > Preview.
- The Markdown preview understands the common HTML in READMEs: centered headings and paragraphs, `<br>`, links, sized images, `<picture>` (showing the dark or light image for the current theme) and `<table>` grids. Scripts, styles and unknown tags are never run or shown.

### Fixed

- A pane could be pushed under the status bar when its neighbour's content (such as a thumbnail grid) did not fit.

### Security

- Remote Markdown images are off by default, never contact private or local addresses, and are limited in size and time.

## [0.1.0] - 2026-11-04

First public release (beta).

### Added

- Web links in the terminal open in your browser with Ctrl+click (Cmd+click on macOS).
- Links in the Markdown preview are clickable. Web and mail links open externally; relative links such as `docs/install.md` open the folder in a Files pane and select the file.
- Markdown files can be viewed as source: the "Preview" toolbar button toggles between the rendered document and the highlighted text (also in the View menu).
- Preview source picker numbers the file panes.
- Thumbnail view can be sorted by name, modified date or type. With subfolders included, files are grouped under a heading per subfolder (switchable to one flat list) and folder tiles are no longer shown.

### Changed

- FileManagerUltra is now licensed under the Apache License 2.0 (previously MIT).
- The log filter bar is shown by default when previewing a log.
- The Diff toolbar shows a single "Diff" toggle for every file except Markdown, instead of a Preview/Diff pair whose Preview button did nothing.

### Fixed

- Thumbnail view in the preview no longer lists hidden files and folders while "Show hidden files" is off.
- Toggling hidden files keeps the cursor on the same file instead of jumping to the top.
- Light theme no longer shows dark bars around the panels.
- The whole path bar, not just the text, can be clicked to edit the path.
- Folders created outside the app (for example by `git clone` in a terminal) now appear in the file tree right away, also under tree folders that are not the pane's current folder.
- Restored workspaces show the folder tree from the filesystem root again, instead of starting at the home folder.
- A failed automatic update check no longer shows an error bar at startup; a manual check explains when GitHub is rate limiting requests.

### Removed

- macOS Intel (x64) release builds; Apple has deprecated the platform.

### Documentation

- macOS install notes: clear the quarantine flag on the downloaded `.dmg` (`xattr -d com.apple.quarantine`), as the app is not yet signed or notarized.

[Unreleased]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/FileManagerUltra/FileManagerUltra/releases/tag/v0.1.0
