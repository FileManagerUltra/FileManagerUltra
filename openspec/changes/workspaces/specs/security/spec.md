## ADDED Requirements

### Requirement: Saved workspaces are private and bounded
`workspaces.json` SHALL hold only pane trees, locations and per-pane view settings — never credentials, terminal scrollback or command history. It SHALL be written atomically with owner-only permissions (`0600`, directory `0700`) through the same writer as `prefs.toml`, and SHALL be size-capped before it is read.

#### Scenario: Oversized or hostile file
- **WHEN** `workspaces.json` exceeds the size cap, is malformed, or holds duplicate or oversize pane trees
- **THEN** unreadable entries are skipped with a notice and the app starts with a built-in single pane, never panicking
