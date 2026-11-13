# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2026-11-12

### Added
- Audio plays faster or slower (0.5× to 3×) without changing pitch, from the Speed menu or the < and > keys, with or without ffmpeg. Each audiobook remembers its own speed; other files share one speed per session.
- Media keys and the system's media controls work with the Media pane: the play/pause, next, previous and stop keys, the GNOME/KDE media widget, the Windows media overlay and macOS Now Playing, with title, artist, album, cover art and position.

### Fixed
- Playing a file while the current item is in a locked playlist folder no longer puts the file inside that folder; it now goes right after it.
- Esc closes the player's right-click menu again, and the arrow keys move through it instead of seeking or changing the volume.
- Videos in a folder (documentary episodes, DVD parts) are no longer treated as an audiobook: earlier episodes are not marked as played, and each video resumes where you stopped it.
- Dropping files or rows on the upper half of a playlist row now puts them above it; before, they always landed below.
- A dropped folder, such as an audiobook or a series, now lands where you drop it in the playlist, as its own playlist folder; before, it was always added at the end.
- Deleting the folder you are looking at (for example from the folder tree) now shows the next highlighted folder, or the parent folder, instead of a "not found" error.
- Splitting a Preview pane now opens a file pane next to it instead of a second preview, and no longer shows the SFTP login when no SFTP pane is open.
- Linux: the `.deb` package now declares what it needs (including the ALSA sound library), so installing it with apt pulls in anything missing instead of failing to start.

## [0.2.0] - 2026-11-12

### Added
- A Media pane that plays local audio and video: a playlist with seeking, volume, mute, shuffle and repeat, a now-playing display with artwork, title and clock, and an Options menu with the equalizer, sleep timer, A-B loop, speed, tracks and gapless playback. Playback can continue after the pane closes or when you switch workspace; starting playback in Preview pauses it.
- Video in Media: sound, seeking, frame-by-frame stepping while paused, speed from 0.5× to 3× with the pitch kept, a choice of audio track, the first frame shown before you press play, a video area you can resize, and fullscreen (F, double-click or the Fullscreen button; Esc leaves; controls hide after 3 seconds). F11 makes the window fullscreen.
- Audio and video files can play inside the file manager on double-click: choose "Play in Media pane" under Preferences → Media → Opening files (separately for audio and video). The first time a media file opens in another app, a short tip offers this once.
- Right-click the player (video, cover or now-playing area) for a menu with play/pause, tracks, subtitles, equalizer, speed, loop, sleep timer, fullscreen and Show in folder.
- Drop files or folders on the player to play them right away, or on the playlist to add them exactly where the line shows; nothing is copied or moved, and folders are added in natural filename order. Added files are selected and scrolled into view. Adding never interrupts what is playing; "Start playing when items are added" in Preferences → Media makes an idle player start.
- Files and Preview offer Play, Play next, Add to queue and Play folder for audio and video. Play inserts after the current item and starts it without discarding the playlist; playing from Preview continues at its current position.
- Play audio and video from SFTP and SMB: the file is downloaded to a private temporary folder first, with progress shown, and removed afterwards.
- Many formats: WMA, Opus, APE, WavPack, AIFF, AC-3, DTS, Matroska audio and more, using ffmpeg when the built-in decoder can't read them.
- DVD disc images (`.iso`) play as one continuous film straight from the image, with no mounting or copying; images that aren't DVD-Video are refused. DVD video (`.VOB`) and Blu-ray/camcorder `.m2ts`/`.mts` files play too, and dropping a DVD folder adds the film's parts in order as one playlist folder named after the disc. Copy-protected discs can't play.
- Subtitles in Media and fullscreen: subtitle files next to the video (such as `movie.en.srt`) are found automatically, and embedded tracks, SRT, WebVTT and ASS/SSA files can be shown. Load another file from the Subtitles menu, and shift subtitles in 0.1 s steps with the menu or the G and H keys. The chosen language carries over to the next video.
- Preferences → Media → Subtitles sets how subtitles look for every format: size, position, font, bold, text colour and opacity, a background box, outline or shadow, and letter and line spacing. Presets such as High contrast and Yellow on black are one click away, and a live preview shows the contrast ratio with a warning when text would be hard to read. Picture-based subtitles (Blu-ray, DVD) are not shown yet.
- Audiobooks: `.m4b` files, files tagged "Audiobook", folders of mp3s under a folder called "Audio Books" or named "Chapter…"/"Part…", and anything you mark with "Treat as audiobook" get Back/Forward skip buttons (15 s / 30 s, adjustable), previous/next chapter, a chapter list, bookmarks, and a seek bar, clock and progress that span the whole book, even across many files. Playing a book or video again continues where you stopped (rewound a few seconds). Rows show a dot for a remembered position and a check mark when played or finished; mark books finished or unfinished from the menu, and clear everything remembered from Preferences.
- Playlist folders: make your own ("New folder" button or right-click menu), drag rows or files onto a folder to add them, drag a row out to ungroup, and rename, delete or lock folders (a padlock stops accidental drops). Adding a folder tree, such as a series of audiobooks, gives each folder with audio its own locked playlist folder named after its path. A folder plays as a playlist inside the playlist, and adding it again or choosing "Resume play" continues where you left off.
- The playlist selects rows like the file list: Shift+arrows, Home/End and Page Up/Down extend the selection, Shift-click and Ctrl-click work the same way, and Delete removes all selected rows.
- Import M3U, M3U8 and PLS playlists, or save the playlist as M3U8. Unsafe links, missing files and unsupported entries are skipped with a message.
- The playlist, playback settings and the current position return after restarting (paused); missing files are skipped with a message.
- Gapless playback between consecutive local tracks (can be switched off), a ten-band equalizer in its own window with built-in and saved presets and an automatic preamp against clipping, an A-B loop, and a sleep timer that fades out before pausing.
- Keyboard controls in Media (Space/K, J/L, arrows, 0–9, Home/End, M, N/P, comma/period, < and >); the Keys menu lists them, and Preview audio and video get the basic transport keys.
- While Media plays (or is paused mid-item), the status bar shows Previous, Play/Pause and Next buttons and a speaker with a volume slider, next to the Activity button.
- Preview plays audio files with Play/Pause, seeking, volume, mute, tags and embedded cover art (autoplay is optional and off by default), and Preview videos now play with sound and synchronized picture. Both use the same controls as the Media pane.
- On Windows, Preferences → Media can download ffmpeg (needed for video): a pinned version, checked against a built-in SHA-256 checksum before it is installed; a copy already on your computer is always used first. When ffmpeg is missing, a dialog offers the download or explains how to install it yourself.
- Find Files: every result has Open, Show in Folder and Copy Path buttons and a right-click menu with Open With, Copy Name and Properties; a running search can be minimised to the Activity panel and keeps going; the last 20 searches are kept under Recent.

### Changed
- The Jobs panel is now called Activity: it holds running jobs, messages, tips, update notices and minimised searches. Each one is a clearly marked card (blue for information, red for errors) with its main action highlighted.
- Find Files moved to Ctrl+F (search the current folder) and Ctrl+Shift+F (search all local drives, skipping system folders such as `/proc`); Alt+F7 is gone. In Preview, Ctrl+F still finds text inside the preview. Log Live tail no longer has a shortcut; use its button or menu.
- Find Files is much faster on local drives (about 9× for name searches and 8× for content searches on a large folder tree) and has a cleaner layout: the name field comes first, results show name, size and folder on two lines, and the window has Minimize and Close buttons.
- Typing in the file listing now opens the quick filter and keeps typing there, instead of jumping to the location bar (use the location bar or Ctrl+L for paths).
- Video in the Preview pane sits in a fixed area (three quarters of the pane), so switching between videos no longer makes the player jump or flicker, and the controls always stay in view.
- Opening a file that is playing in the Preview pane (double-click, Enter or Play) pauses the preview, so it doesn't keep playing alongside the other player.
- In a diff, Previous/Next change and Find bring the result to the middle of the view, and the current change and Find match are clearly outlined. The diff's Find bar uses Previous/Next buttons like the Preview's.
- Show in Folder scrolls the folder so the item sits in the middle of the list.
- Copy and move progress ("Copy 1 item to Videos · 44%") now sits at the right of the status bar, next to the Activity button.

### Fixed
- Git markers in the file list now clear after a commit, including commits made through a pre-commit hook; before, staged files could stay marked as changed.
- Informational messages, such as "Opening … in your default browser", are no longer labelled as errors.
- Selecting another image in the Preview pane sometimes kept showing the previous image, stretched to the new image's shape.
- Widescreen videos stored with non-square pixels (many DVD and broadcast files) are no longer squeezed to 4:3 in Preview.
- A server opened by address in the Network pane (such as `\\192.168.0.5` or `smb://192.168.0.5`) now shows in the folder tree, even when it doesn't announce itself on the network.
- Windows: Go to Path completes folders with backslashes (`C:\Users\Documents\`) instead of mixing in forward slashes.
- Shift+Arrow keyboard selection keeps its starting row instead of sliding after two files.
- Ctrl-clicking an item to deselect it no longer leaves it looking selected.
- Symbolic links that point to folders (such as `bin` -> `usr/bin`) now show in the folder tree.
- Folders no longer get stuck on "Loading…" while a large copy or move is running, and they show new items during the copy instead of only after it.

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

[Unreleased]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/FileManagerUltra/FileManagerUltra/releases/tag/v0.1.0
