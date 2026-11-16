# directory-cache Specification

## Purpose
TBD - created by archiving change multi-pane-ui. Update Purpose after archive.
## Requirements
### Requirement: Directory reads land in a shared cache
The engine SHALL cache directory listings by normalized `VfsUri` as `(revision, Arc<[Entry]>)`; every pane showing that uri reads from the same cached entry.

#### Scenario: One read serves two panes
- **WHEN** two panes are navigated to the same directory
- **THEN** at most one `ReadDir` job is in flight for that uri and both panes render from the same cached `Arc<[Entry]>`

#### Scenario: Revisions are monotonic
- **WHEN** a directory is re-read (refresh or invalidate)
- **THEN** its revision strictly increases and consumers can tell stale from fresh

### Requirement: Reads travel the job lanes
A pane navigation SHALL submit a `ReadDir` job to the lane matching the uri's scheme and return immediately; results and failures SHALL be applied only when the UI drains events in `Session::poll`.

#### Scenario: Navigation does not block the frame
- **WHEN** `nav.open` runs for a slow directory (test: a MemFs backend that sleeps)
- **THEN** the command returns `Ok` before the entries are available and the pane shows a loading state

#### Scenario: Failed reads are visible, not swallowed
- **WHEN** the read job fails (permission denied, vanished directory)
- **THEN** the cache records the failure for that uri and the pane renders an error state with the message and a retry action

### Requirement: Cache invalidation is total, never patched
A watcher or refresh SHALL invalidate a uri's cache entry (bump revision, drop entries); the next read re-fetches the whole listing rather than applying incremental patches.

#### Scenario: Refresh re-reads from the backend
- **WHEN** `nav.refresh` runs for the focused pane
- **THEN** the pane's current uri is invalidated and re-read through a job, and a new revision replaces the old entries atomically

### Requirement: Visible tree folders stay live
Local folders whose contents are on screen SHALL be watched for external changes: the Files panes' current folders and every expanded node of their trees. Pane folders SHALL take priority when the watch cap is reached.

#### Scenario: External folder under an expanded tree node
- **WHEN** another program creates a folder inside a tree node that is expanded but is not the pane's current folder
- **THEN** the new folder appears under that node without the user refreshing

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

