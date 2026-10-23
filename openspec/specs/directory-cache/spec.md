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

