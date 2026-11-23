# Changelog

All notable changes to FileManagerUltra are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Hold Ctrl and turn the mouse wheel over a pane to zoom that pane in or out, one step per notch; a trackpad pinch works too. It zooms the pane under the pointer, also when it is not the active one, just like the zoom button in the pane's header.
- CSV and TSV files open as a table in the Preview pane, with a fixed header row, row numbers and columns you can resize. Click a column header to sort (Shift+click adds a second or third column); numbers sort by value. Filter rows by text in all columns or one, optionally as a regular expression, and add conditions such as "greater than" or "is empty" from a column's menu. The Record panel shows the selected row with every value in full, which helps with long descriptions. Copy a cell, a row or the column names, go to a row number, and hide columns you don't need. The delimiter and header row are detected automatically and can be changed in the toolbar. Large files open straight away and keep loading in the background, so files of several gigabytes stay quick, also on network shares and SFTP. The file's text is one click away.

### Changed

- The Signatures panel explains why a signature is broken. A PDF that was merged with other PDFs or re-saved by another program after signing now says so, and shows how much of the current file the signature covers, instead of calling the signature malformed. Broken signatures still show who signed and when, self-signed certificates are marked as such, and the certificate details include its subject and validity period.

### Fixed

- Photos larger than 2048 pixels are sharp at 100% and when zoomed in, instead of an enlarged, blurry copy of the smaller version made for Fit. The full-size picture loads in the background the first time you need it (up to 8192 pixels on the longest side).
- Zooming a pane, terminal or preview all the way out and back in now returns to exactly 100%; before, it could end up at 104%.
- Workspace tabs now reopen in the order you dragged them into; before, they came back in the order they were first saved.
- The Open With window no longer grows taller than the screen on systems with many installed applications; the list scrolls inside the window and the Open and Cancel buttons stay in view.
- The Synchronize Folders window no longer grows past the edge of the app window on small screens or with large text, which could leave its title bar, Minimize and Close out of reach. When the window cannot fit, its contents scroll and the list gets smaller, so the Back, Next and Start buttons stay in view.
- Opening a folder you have no permission to read no longer also adds a "Live refresh is unavailable" error to the Activity panel; the Files pane already says access was denied.

## [0.3.1] - 2026-11-20

### Added

- Files panes have Back and Forward arrows before the path, and the mouse's back and forward buttons work over any Files pane. History lasts until you quit or close the pane.
- PDFs open in the Preview pane: pages scroll continuously, with page numbers and labels, bookmarks, working links, Find, text selection and copy. Pages can fit the width, fit the page or show at 100%, and can be shown as dark pages or in high contrast for easier reading (the file itself is not changed). Password-protected PDFs ask for the password, which is kept only while the document is shown. Folder thumbnails show the first page of PDFs.
- Fill & Sign for PDFs, from the Fill & Sign button at the top right of the pages: large on forms (it shows how many fields they have), on other PDFs a coloured Fill & Sign button at the right end of the preview bar, always visible, so nothing covers the pages. It opens a column of coloured action buttons below it (Signature, Place, Remember, Reset, Save copy, Save as, Replace, Discard), which you can fold away by clicking Fill & Sign again: fill in form fields (text, checkboxes, radio buttons, lists), add a drawn, typed or image signature and place it on a page or in a signature field, or sign digitally with a certificate file (.p12/.pfx). Nothing is written until you save: Save copy puts "… (filled).pdf" or "… (signed).pdf" next to the original, or use Save as or Replace original (refused if the file changed meanwhile). Quitting or closing the pane with unsaved changes asks first. Signatures you make can be remembered on this computer. Forms that rely on scripts (for example calculated totals) suggest using a PDF application.
- Signed PDFs show whether their signatures are intact, changed after signing or broken, with signer, time and reason, in a Signatures panel. The signature state also shows as a square on the pages straight away (green when intact, amber when changed after signing, red when broken); clicking it opens the panel.
- The "Unsaved workspace changes" question has a "Don't ask again" checkbox: tick it and your Save or Don't save answer is used from then on when quitting or closing a tab. Change it back under Preferences › Workspaces › Unsaved changes.

### Changed

- PDFs no longer show the Diff button, which only made sense for text.
- The Preview pane shows the file name in its header (the full path when you hover it), so the bar below has room for the buttons.
- Third-party licences have their own window, Help ▸ Third-party Licences (also reachable from About), so About stays short. The list now includes the fonts and character maps the PDF viewer uses.
- Only changes to a workspace's set-up (panes, splits, sort, column widths, zoom and such) count as unsaved. Going to other folders, selecting files or expanding the tree no longer does: a workspace reopens at the folders you saved it with, unless Auto-save is on.
- Back/forward history is no longer saved with workspaces.

### Fixed

- Stopping a Find Files search that also looks inside files now takes effect at once, instead of finishing the folder it was in first.
- Saved workspaces no longer count as changed right after opening, so quitting without touching anything no longer asks to save them.
- Closing the last tab opens your default workspace again, as intended.
- Synchronize Folders: after resizing the window the list no longer slowly shrinks, and on a small screen the window now fits instead of running off the top and bottom.
- Synchronize Folders (Advanced): pressing Change settings with a row selected no longer makes the window grow off screen; the row's details return when you hide the settings.
- Synchronize Folders: turning on Keep in sync no longer puts a red card in the Activity panel while the window waits for you to press Start syncing. If you minimize the window at that point, the card now says it is waiting for Start syncing instead of looking like an error.
- Synchronize Folders: the Start button of Keep in sync no longer disappears after switching between Guided setup and Advanced, or after pressing Stop syncing; the review said the folders already match even when there were changes.

## [0.3.0] - 2026-11-19

### Added
- Synchronize Folders (File menu, starting from the folder you have highlighted): compare two folders (local, network share or SFTP), review what would be copied or deleted, change any item's action, then synchronize. Choose Update (copy new and newer files), Mirror (make one folder identical to the other) or Two-way (changes go both ways, conflicts are left for you). Deletes that cannot be undone ask for confirmation first. A target folder that does not exist yet is created by the first sync. The window can be minimized to the Activity panel with a status in the status bar (also when you close it while "Keep in sync" is on), and the grid can be sorted by name, size, modified time or action. Profiles can be saved, and "Keep in sync" first compares and asks you to confirm what it will do, then re-compares and applies safe changes while the app is open (conflicts and deletes wait for your review); it can be stopped from the Activity panel, and folders are shown as normal OS paths. A new sync starts in a guided setup that asks a few questions (folders, goal, deleting, comparing, what to ignore, keep in sync) and ends with a review of the changes before anything happens; an Advanced view offers every option at once, and both explain what each button does. Both views show the same choices (changing one updates the other), use the same icons and colors, and have a button to swap the two folders; the guided setup saves the profile on the last step and shows the source and target folders above the list of changes. Deletes that cannot be undone ask in their own warning dialog. While syncing, the window and the Activity panel show the amount done, speed, elapsed and remaining time. In Advanced, "Keep in sync" now ends in "Start syncing" and a "Minimize to Activity panel" button.
- The Browse… chooser has a sidebar with your places, bookmarks, devices, the file system (This PC on Windows), Network and Saved servers for faster browsing; picking a server that is not connected yet opens its sign-in dialog right from the chooser.
- Browse… buttons choose a folder or file from a list (local, network share or SFTP) instead of typing it: on both sync folders, the copy/move destination and the subtitle and playlist dialogs.

### Changed
- Quitting while a copy, move, delete or folder sync is still running asks first ("Keep running" or "Quit anyway"). Closing the window with its close button now asks the same questions as File ▸ Quit, including about unsaved workspaces.
- macOS: the disk image and the app are now signed with an Apple Developer ID and notarized by Apple, so they open without Gatekeeper warnings or quarantine workarounds. The app has a new identifier (`com.filemanagerultra.mac`), so macOS asks once more for folder permissions you granted before; settings are kept.
- The folder chooser is tidier: Up, New folder and a new Show hidden button are icon buttons with tooltips, the path box lines up with them, and the places list runs the full height.
- Synchronize Folders: the action filter in the list is cleared whenever you start a new comparison.
- Synchronize Folders: the Advanced view now matches the guided setup: Save and Synchronize buttons look the same and sit below the list (Synchronize last), and the comparison summary sits in the same always-visible footer with the expand button.
- Synchronize Folders: the "Name and save this sync" card has a large save illustration so it stands out.
- Synchronize Folders: a "Cancel scan" button next to the scanning progress stops a long comparison.
- Synchronize Folders: the action menu of an item explains each choice (Skip, Do nothing and the rest) in a tooltip and offers "Add to ignore list", which adds the file or folder to the sync's ignore list (the "Leave out" step) and compares again.
- Synchronize Folders: the Action column has a filter (funnel icon) to show only items that will be copied either way, deleted on either side or skipped, the Action column shows clear icons (copy either way, delete on either side, skip, no action, identical, conflict) instead of text, with the same icons and labels in its menus, and the table's "Expand" button is now a square icon next to the comparison result.
- The last step of the guided sync setup no longer shows the purple notice; the comparison result (and any permanent-delete warning) now sits in its own bar below the list, where it can't be scrolled out of view.
- The SFTP key file "Browse…" uses the same file chooser as everywhere else; choosers opened from a connect dialog open as separate windows stacked in order.
- The Browse… chooser is smaller and denser, and has a New folder button to create a folder where you are browsing.
- The sync list is now a folder tree: click a folder to fold or unfold it (it shows how many files are inside and what will happen to them), long names are shortened in the middle, hovering a row shows its full path, and clicking a row shows the full left and right paths below the list.
- Folder sync: a folder's menu applies a choice (copy, skip, delete…) to everything inside it, “Expand table” opens the list of changes in a large window (Esc closes it first), and after a comparison the Advanced settings fold into one line (“Change settings” brings them back) so the list gets more room.
- The Advanced folder sync view now follows the same order as the guided setup (folders, goal, deleting, compare by, leave out, keep in sync) on one page, in a window the same size as the guided setup, with Synchronize in the far corner.

### Fixed
- Copying or synchronizing many files to an SFTP server no longer fails part-way with "Invalid file handle" on servers that use binary file handles (such as asyncssh-based ones); every file now arrives.
- A preferences file with one missing or mistyped setting (for example after editing it by hand) no longer resets every setting to its default; only that setting falls back.
- Ctrl+R in a Preview pane no longer shows an "unsupported" error; it refreshes the folder the preview is showing.
- Synchronize Folders now starts from the folder you selected (or the current folder), not the one the cursor happens to rest on.
- The large "Changes" window now fills its whole table, and opening it from the guided setup no longer crashes the app.
- Switching a pane (for example a preview pane restored from an earlier session) to a Files pane no longer pops up an uncancellable connection dialog for a server you are not connected to; it opens the folder selected in the other pane.

## [0.2.4] - 2026-11-16

### Added
- Drive details everywhere: selecting a drive (such as C: in This PC) or any other volume root shows its name, filesystem, capacity, free space and a usage bar in the Preview pane, and Properties of a drive shows the same right away instead of counting every file on it. Properties of any other item shows how much space is free on the drive or server it is on.
- Bookmarks and places on a network share (SMB) or SFTP server open in a Network or SFTP pane of their own (an open one is reused, otherwise one opens beside the current pane), so the Files pane you clicked in keeps its folder and tree. Signing in, when needed, happens for that pane.
- Mermaid diagrams in Markdown: ```` ```mermaid ```` code blocks in the Preview are drawn as diagrams (flowcharts, sequence, class, state, ER, Gantt, pie, journey, timeline, git graph and mind map) in the app's colours, so they follow dark/light and your accent and stay readable. They are drawn offline and redrawn sharp when you zoom. A diagram that cannot be drawn stays a code block with the reason above it. Each diagram has buttons to copy its source, copy it as a picture (to paste into a document or chat), and show its code instead of the drawing. Turn it off in Preferences ▸ Preview ▸ Draw Mermaid diagrams; Preferences ▸ Advanced ▸ Clear diagram cache draws every diagram again.
- Help ▸ About has a cleaner layout and lists the third-party parts that come with their own licence (the reading font and the diagram layout engine); View licence shows the full text.
- Find Files now searches network shares (SMB), by name and by content, like SFTP servers and local folders. Properties now shows the size and number of items of folders on network shares too. In the Network folder or on a server that lists its shares, Find Files tells you to open a share first.

### Changed
- Folder sync has an “Open saved” button next to square Guided setup and Advanced buttons with icons, and the wizard's last step keeps the Start button in the far corner, away from Back.
- The sync wizard has a clear header for each question, outlined folder boxes, the same pattern list as Preferences for “Leave out”, and one fixed size so Back and Next never move between steps.
- Dialogs are roomier and easier to read: larger text, taller buttons and list rows, clearly outlined buttons and input fields, and better contrast in the dark theme. The folder chooser is larger to match.
- Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 now always zoom the whole app, also when a terminal or Preview has focus, so the keys do the same everywhere. To zoom only one terminal or Preview, use the zoom button in its header (a Preview also keeps Ctrl+= / Ctrl+- / Ctrl+0).
- The built-in audio decoder is updated. Damaged or deliberately malformed audio files are handled more safely, MP3 files that only carry old-style (ID3v1) or APE tags now show their title and artist, and some variable-bitrate MP3s report their correct length.
- Drives in This PC are shown as Windows writes them (`C:`, `Data (D:)`), without a trailing slash.
- Paths are shown the way your system writes them, everywhere: on Windows `C:\Users\you`, `\\server\share\folder` and This PC instead of `file:///C:/…`, on Linux and macOS `/home/you`. This covers the location bar, recent locations, Find Files (including Search everywhere), bookmark names, tooltips, Properties, Copy Path and the Copy/Move destination. SFTP folders always read `sftp://user@host/folder`. Any path the app shows can be typed or pasted back; on Windows a `\\server\share` path uses the app's own connection to that server when one is open.

### Fixed
- Playing audio or video no longer makes the app redraw about 30 times a second in the background. A 25 fps video now draws about 25 frames a second, and with the player hidden or the window minimized the app wakes only a few times a second, which saves power on laptops.
- Clicking the folder path in a pane's location bar starts editing it. Before, only a click beside the text worked.
- A new terminal starts where you were working: splitting a terminal opens the new one in that terminal's folder (on Linux even after a `cd`), otherwise it uses the active pane's folder or the last folder you used in any workspace, instead of an old folder the pane once showed. Restart Shell keeps the folder.
- Opening a bookmark (or another saved location) on an SMB share that isn't connected now asks you to sign in and then opens it, instead of failing with "no backend registered for this location".
- Bookmarks of a whole server are no longer named by their raw address (`smb://192.168.0.50/`).

## [0.2.3] - 2026-11-15

### Added
- Every pane has its own zoom: click the zoom button (a magnifying glass with a plus) next to the pane type in the pane's header for zoom in, the percentage (click to reset) and zoom out. Files, Network, SFTP, Trash and Media panes get larger or smaller text and rows (in the Media pane the player buttons, seek bar and playlist too); other panes and the rest of the window keep their size, and the zoom is saved with the workspace. While a pane is zoomed, its button shows the percentage. The button works from the keyboard too (Tab to it, Enter to open, Escape to close), has large click targets and is labelled for screen readers.
- Each terminal has its own zoom (the header zoom button, or Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 while it has focus); other terminals keep their size.
- Ctrl+Tab and Ctrl+Shift+Tab switch between workspace tabs, also from a terminal. By default the first press returns to the tab you used before; keep Ctrl held and press Tab again to go further back. Preferences > Workspaces can switch to plain left-to-right order instead.
- The status bar shows the playing time (elapsed / total) next to its media controls, so you can see where playback is with the Media pane closed or another workspace in front.
- A diagnostic log for bug reports: turn on Preferences > Advanced > Troubleshooting > Write diagnostic log. Once a second it records the app's memory use (including graphics memory) and what it was doing, such as playing video or opening panes. It never records file names, folder names, server names or passwords. Open log folder (also in the Help menu) shows the logs; each session's log stops at 64 MiB, and the last 20 are kept.
- About shows which build you are running, so test builds can be told apart.

### Changed
- In a terminal, Ctrl+Shift+Tab now switches to the previous workspace tab instead of swapping panes; use the Panes menu to swap panes from a terminal.
- Playing video redraws the window at the video's own frame rate instead of 60 times a second, and not at all while the player is hidden or the window is minimized, which saves power.
- Thumbnails now keep at most 128 MiB in memory by default (was 192 MiB). Change it in Preferences > Advanced > Thumbnails if you browse very large picture folders.
- Loading spinners (folder sizes, previews, searches, jobs) redraw about 15 times a second instead of at the screen's refresh rate, which saves power while something loads.
- While idle the window no longer redraws several times every two seconds while checking for new drives; it redraws only when the check is done.
- Windows: the app's settings folder in `AppData\Roaming` is now called `FileManagerUltra`, the same as its folder in `AppData\Local`. An existing `filemanagerultra` folder is renamed when the app starts; your settings, workspaces and playlists stay.

### Fixed
- Show in Folder (from Find Files or the playlist) no longer fails in a workspace without a Files pane, such as one holding only a terminal; it opens a Files pane beside it.
- Ctrl+Shift+1 now switches to the first workspace tab, like Ctrl+Shift+2 to 9 already did, and Ctrl plus a digit works on keyboard layouts that need Shift for digits.
- Windows: File > Open With… did not open; it now shows Windows' own "How do you want to open this file?" dialog.
- Windows with an NVIDIA graphics card: the app's memory no longer grows while its window is visible (about 1 MB a second while playing video or moving the mouse over it, up to many gigabytes over a long session). The window is now drawn with Direct3D 12 instead of OpenGL, so changing the driver's "Threaded optimization" setting is no longer needed. If Direct3D cannot start (older or basic graphics cards), the app uses OpenGL and says so once; on a computer without a usable graphics card at all it still opens, drawing in software. Preferences > Advanced > Troubleshooting > Renderer switches between Direct3D and OpenGL (after a restart).
- Seeking or restarting a video keeps its picture buffer on the graphics card instead of making a new one each time.

## [0.2.2] - 2026-11-13

### Added
- Camera RAW photos (DNG, CR2, NEF, ARW, ORF, PEF and similar) are shown in Preview and as folder thumbnails, using the preview picture the camera stored inside them. Copy Image copies that picture.
- Pictures in Preview have Rotate left and Rotate right buttons. Only the view turns; the file is not changed, and the next picture starts upright.
- Right-click a playlist row and pick Show in folder to open its folder in a Files pane with the file selected. A playlist folder made from a folder on disk offers Go to folder on disk.
- SVG files are shown as pictures in Preview, including their text, with Fit and 100% and a choice of background behind transparent parts (checkered, white, black or 50% grey, remembered). Turn off Preview in the header to read the SVG source (with Pretty and Wrap). SVG is a new type in Preferences > Preview > File types; existing settings pick it up automatically.
- SVG drawings, including compressed `.svgz` files, show as pictures in folder thumbnails, over the same background.
- Picture-based subtitles from Blu-ray rips (PGS) and DVDs (VobSub) can now be shown in the Media pane. They follow the subtitle Size and Distance from bottom settings; switching them keeps the sound playing.
- Interlaced video, such as DVDs and TV recordings, is deinterlaced automatically, so moving edges no longer show comb lines. This applies to playback, poster frames and thumbnails; other videos are not changed.
- Videos are decoded on the graphics card where possible (VA-API on Linux, Direct3D 11 on Windows, VideoToolbox on macOS), which uses less power and plays large videos more smoothly. If the graphics card cannot decode a video, it plays in software without interruption. Turn it off in Preferences > Media > Playback; the player's menu shows which one is used.
- Preferences can be used with the keyboard alone. Tab moves through the topic list, the topic's settings and Close, and stays inside the window. Up and Down in the topic list, or Ctrl+Tab and Ctrl+Shift+Tab anywhere, switch topics. Choices such as theme, accent colour and density are a single Tab stop whose options you pick with the arrow keys. The focused control is ringed and scrolls into view.
- In Preferences > Preview > File types, the arrow keys work in both lists: Up and Down move through types or patterns; Tab, Shift+Tab, Left and Right switch between the two lists.

### Changed
- Media fullscreen (F, or a double-click on the video) now fills the whole screen instead of only the app window. Leaving it puts the window back as it was.
- Renaming a workspace tab or a bookmark starts with the current name selected, so typing replaces it.
- Dropping a file on the Media player that is already in the playlist no longer adds it a second time: its row is selected and it starts playing (a file that is already playing just keeps playing).
- Text in SVG images inside Markdown documents is now drawn.
- Preferences > Preview > File types is easier to use: pick a type in the list on the left to see its filename patterns on the right, then add (+), remove (−) or edit (pencil, a second click, or F2) them in place. Up and down arrows change which type is checked first. In Preferences, Esc in a text field now leaves the field first instead of closing the window.
- The subtitle Font and Edge style in Preferences > Media are now option rows instead of dropdowns, so every choice is visible at once.

### Fixed
- Closing a workspace, a pane or the Media pane now gives back the memory its pictures, video frames and thumbnails used.
- Playing a video no longer makes the app's memory use grow steadily on Windows: each new frame now reuses the same picture on the graphics card instead of making a new one.
- Photos from phones and cameras are shown upright in Preview, folder thumbnails and Markdown documents, following the orientation the camera stored. Thumbnails made earlier are redrawn.
- Videos filmed upright on a phone no longer play stretched sideways: they are shown upright and tall, as on the phone.
- Arrow keys pressed in Preferences no longer also move the cursor in the file list or folder thumbnails behind it.

### Security
- An SVG (in Preview or as an image in a Markdown document) can no longer make the app read other files from your computer through image references.

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

[Unreleased]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.4...v0.3.0
[0.2.4]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.3...v0.2.0
[0.1.3]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/FileManagerUltra/FileManagerUltra/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/FileManagerUltra/FileManagerUltra/releases/tag/v0.1.0
