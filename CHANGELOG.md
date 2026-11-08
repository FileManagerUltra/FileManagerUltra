# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/FileManagerUltra/FileManagerUltra/releases/tag/v0.1.0
