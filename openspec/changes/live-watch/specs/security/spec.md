## ADDED Requirements

### Requirement: Watcher input is untrusted and bounded
Watcher events and paths SHALL be normalized and bounds-checked before mapping to a `VfsUri`; the number of concurrent watches and queued events SHALL be capped, with overflow degrading to invalidate/poll rather than unbounded memory growth.

#### Scenario: Hostile event flood
- **WHEN** a directory emits events faster than they are drained
- **THEN** memory stays bounded and the directory is invalidated at most once per debounce window

#### Scenario: Event for an unexpected path
- **WHEN** an event path cannot be mapped to a displayed `VfsUri`
- **THEN** it is ignored without error or panic
