## ADDED Requirements

### Requirement: Diagnostic log
Preferences SHALL offer a "Write diagnostic log" switch, off by default, and an "Open log folder" action (`app.diagnostics.folder`). While the switch is on, the application SHALL append to a text log in its data folder (`diagnostics/diagnostics-<timestamp>.log`, one file per session) containing: a startup summary (application version, build commit when known, operating system, OpenGL vendor, renderer and version as reported by the renderer, display scale and the video decoding settings); one sample per second with working-set memory, commit size (private bytes), dedicated GPU memory when the platform reports it, egui texture count and bytes, frames drawn, video frames received and video texture uploads by kind (created, resized, in place), frame-cache evictions, running video decoders, decoded video frames alive and their buffer bytes, running ffmpeg/ffprobe helpers, thumbnails made and loaded from disk, pending thumbnail jobs, preview renders started, whether the Media video plays while no Media pane is shown, panes in total and per kind, workspaces, preview slots, thumbnail memory, entries in egui's memory, frames that had input, frames without a requested repaint, and the call sites (file and line) that requested the second's repaints; and events for video texture creation and resizing, decoder starts (width, still or playing, hardware, subtitles), dispatched command ids and warnings from the graphics stack. Turning the switch on during a session SHALL include the startup summary, including renderer lines captured at startup. All file writing and memory sampling SHALL happen off the UI thread; when the switch is off the hooks SHALL cost no more than an atomic load.

#### Scenario: Measuring a memory leak
- **WHEN** the user turns on the diagnostic log, plays a video for a minute and opens the log folder
- **THEN** the log has a startup summary naming the OpenGL renderer and about sixty sample lines whose memory and texture columns show how each changed

#### Scenario: Off by default
- **WHEN** the application runs with default preferences
- **THEN** no diagnostics folder or log file is created

### Requirement: Diagnostic log is bounded
A log file SHALL stop growing at 64 MiB, ending with a line saying so. Events SHALL be limited to 200 lines per second, with the number of dropped lines reported in the next sample. Log records SHALL be queued through a bounded channel that drops (and counts) records instead of blocking the caller. At most 20 log files SHALL be kept; older ones are deleted when a new log starts.

#### Scenario: Event storm
- **WHEN** a code path emits thousands of events per second
- **THEN** at most 200 are written per second, the next sample reports how many were dropped, and the UI frame rate is unaffected
