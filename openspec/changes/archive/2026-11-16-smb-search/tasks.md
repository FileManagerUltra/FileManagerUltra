## 1. Engine

- [x] 1.1 `search_cmds::searchable(uri)`: local, SFTP and SMB-with-share pass; Network, Servers and an SMB server root fail with their messages
- [x] 1.2 `search_cmds::start` uses it; SMB goes through the generic `Walker`
- [x] 1.3 `props_cmds::start` walks every `searchable` item (SMB shares included)

## 2. Tests

- [x] 2.1 Find Files on an in-memory SMB share finds files in subfolders (name and content)
- [x] 2.2 Network folder and SMB server root are refused with the new messages; no search starts
- [x] 2.3 Properties counts a folder on an in-memory SMB share without the "not counted" note

## 3. Docs and wrap-up

- [x] 3.1 AGENTS.md §7 and docs/architecture.md §7: walks may use the SMB session from their own thread
- [x] 3.2 CHANGELOG, devlog; test/clippy/fmt/layers/brand-check
- [x] 3.3 Manual: search a NAS share by name and by content; Properties on a share folder (passed, user check 2026-11-16)
