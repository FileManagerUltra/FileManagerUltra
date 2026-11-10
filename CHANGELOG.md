# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Preview's audio and video players now use the same icon controls, seek bar and volume as the Media pane. The player in Preview is a larger card shown above the cover art. The Media equalizer is redesigned and opens in its own window that stays open while you pick presets, and the right-click menu only shows audiobook entries where they apply.

### Added

- Media playlist rows with a remembered position show a small half-filled dot in the gutter ("Resumes at 12:34" on hover), and finished books show a check mark.
- Media remembers where the current item stopped when you close the app: after reopening, the player shows that position and length, and Play continues from there.
- Audiobooks in Media: `.m4b` files, files tagged "Audiobook", and folders or files you mark ("Treat as audiobook" in the right-click menu) get Back/Forward skip buttons (15 s / 30 s, adjustable in Preferences), previous/next chapter, a chapter list, bookmarks, and a seek bar and progress percentage that span the whole book, even across many files. Playing a book or video again continues where you stopped (rewound a few seconds); a book counts as finished near its end and then starts over. Mark books finished or unfinished from the menu, and clear everything remembered from Preferences.
- Make your own folders in the Media playlist ("New folder" button or right-click menu): drag playlist rows or files from the Files pane onto a folder header or any row inside it to add them there (dropping Files-pane files on playlist rows used to be ignored), drag a row out to ungroup, and rename or delete folders. A folder is a playlist inside the playlist: playback stops at its end and is ready at its first item. Folders are remembered between sessions.
- The Media playlist has icon buttons (New folder, Clear playlist) on the right.
- Audiobooks whose AAC header has a quirk (such as some `.m4b` files that failed with "unexpected end of bitstream") now play.
- Dropping or adding a folder tree to Media (such as a series of audiobooks) scans its subfolders: each folder with audio files, even a single .m4b, becomes its own playlist folder named after its path ("Series / Book 1"), and each one remembers where you left off.
- Files inside a Media playlist folder are indented under the folder header with a guide line and numbered from 1 within the folder.
- Files dragged from the Files pane into the Media playlist land where the line shows (after a row, before or after a folder) instead of at the end.
- Lock a Media playlist folder (right-click its header) so nothing can be dropped in or taken out by accident; a padlock marks it. Folders from added directories, such as audiobooks, start locked. Locked folders can still be moved, renamed, played and deleted.
- In the Media playlist, drag a row or a whole folder onto a row to place it right after that row (the top of the first row puts it at the very top, the bottom of a folder's last row puts it after the folder), onto the top of a folder name to place it before the folder, or onto the folder name to make it the folder's first item (a collapsed folder's lower part places it after the folder), and select several rows (Ctrl/Shift-click) to remove them with the menu or the Delete key.
- Folders added to Media (such as an audiobook) are grouped under a collapsible folder header, and adding that folder again, or double-clicking its header (or choosing "Resume play"), continues at the file and position where you left off.
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

### Changed

- A video selected in the Media playlist now shows its first frame before you press play, instead of a black view.
- The video area in Media can be resized by dragging the handle below it; the picture refits to the new size.
- Redesigned the Media pane in a Winamp style: a now-playing display with artwork, title and clock, an icon-based control row with tooltips, a numbered playlist, and an Options menu holding the equalizer, sleep timer, A-B loop, speed, tracks, gapless and other settings.

### Fixed

- In a Media audiobook folder, the files before the one you are on now show a check mark ("Played") in the gutter.
- Folders of mp3 audiobooks are now recognised as books (and get resume markers, chapters and skip buttons) when they sit under a folder called "Audio Books" or most files are named "Chapter…"/"Part…"; use "Treat as audiobook" for any other folder.
- Resuming an audiobook or track no longer flashes back to 0:00 (and no longer overwrites the remembered position), and an audiobook's clock now shows the time in the whole book, matching the seek bar.
- Symbolic links that point to directories (such as `bin` -> `usr/bin`) now show in the folder tree.
- Folders no longer get stuck on "Loading…" while a large copy or move is running.
- Folders now show new items while a copy or move is still running, instead of only after it finishes.
- Ctrl-clicking an item to deselect it no longer leaves it looking selected.

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
