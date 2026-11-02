## ADDED Requirements

### Requirement: Visible local directories refresh automatically
The engine SHALL watch every local directory displayed by a pane, non-recursively, on Linux, macOS and Windows, and SHALL invalidate that directory's cache entry when it changes. Events SHALL be debounced (100 ms) and SHALL NOT trigger a read inline.

#### Scenario: External file creation appears
- **WHEN** another process creates a file in a displayed local directory
- **THEN** the pane lists the new file within one second without user action

#### Scenario: Event burst causes bounded work
- **WHEN** 10 000 change events arrive for one directory within a second
- **THEN** at most one `ReadDir` job is in flight for it and the UI thread never blocks

#### Scenario: Watch buffer overflow
- **WHEN** the platform reports an overflow or rescan event
- **THEN** the whole directory is invalidated and re-read

#### Scenario: Watched directory vanishes
- **WHEN** a watched directory is deleted or its volume disappears
- **THEN** the pane shows an actionable error state and no panic occurs

### Requirement: Watchers follow visibility
Watches SHALL be reference counted by displayed URI and SHALL be added or removed as panes navigate, split, close or workspace tabs switch; hidden tabs hold no watches.

#### Scenario: Two panes share one watch
- **WHEN** two panes show the same directory and one navigates away
- **THEN** the watch remains until the last pane leaves

### Requirement: Polling fallback where events are unavailable
Remote (SFTP, SMB) directories SHALL be revalidated every 5 s while visible (configured remote poll) and on window focus regain. Local directories without reliable native events (Windows mapped/UNC drives, network filesystems), over the watch cap, or whose watch cannot be created SHALL be revalidated every 2 s while visible. Polls run on the matching job lane and an unchanged listing SHALL NOT bump the revision.

#### Scenario: Watch limit reached
- **WHEN** the OS refuses a new watch (for example inotify limit)
- **THEN** a notice is shown and that directory falls back to polling

#### Scenario: Recover a failed watch
- **WHEN** watcher initialization or a directory watch fails temporarily
- **THEN** polling continues and watch creation retries after 1, 2, 4, 8, 16, 32 and then at most once every 60 seconds
- **AND** repeated failures in the same retry sequence do not repeat notices every frame
- **AND** successful watch creation ends the retry sequence without requiring navigation
- **AND** leaving a directory discards its pending retry
- **AND** directories over the watch cap become eligible when a watch slot is freed, while known network filesystems continue polling without failed-watch retry loops

#### Scenario: Unchanged poll
- **WHEN** a poll returns identical entries
- **THEN** the revision is unchanged and the pane does not redraw

### Requirement: Refresh preserves pane state
When a refresh replaces entries, selection and focus SHALL be retained by entry name where the entry still exists, and scroll position SHALL be clamped.

#### Scenario: Selected file survives unrelated change
- **WHEN** a different file is added while one is selected
- **THEN** the same file stays selected
