## ADDED Requirements

### Requirement: Dragging local files out to other applications
When an in-app drag of local files leaves the window with the primary button held, the application SHALL continue it as a native OS drag carrying those files, on Linux (X11 and native Wayland) and on Windows, so other applications can receive them. Remote items cannot be dragged out and SHALL produce a notice instead. The payload SHALL contain only absolute local paths, capped in count, and SHALL NOT stage, copy or delete any file. A failure SHALL surface as a notice and never as a crash. On platforms without a drag source (macOS) the application SHALL report that dragging out is not supported.

#### Scenario: Drop files into another Windows application
- **WHEN** the user drags selected local files out of a pane on Windows and releases over a window that accepts file drops
- **THEN** that application receives the files, the files stay where they are, and the status bar reports "Dropped files into another application"

#### Scenario: Cancel the OS drag
- **WHEN** the user presses Escape during the OS drag, or releases over a window that takes no files
- **THEN** nothing is dropped, no failure notice is shown, and the next click in the window works normally

#### Scenario: Remote items are not dragged out
- **WHEN** the dragged items include a remote (SFTP/SMB) item and the pointer leaves the window
- **THEN** no OS drag starts and a notice says only local files can be dragged out

#### Scenario: Unsupported platform
- **WHEN** a drag leaves the window on a platform without a drag source
- **THEN** the application shows a "not supported" notice and does not crash
