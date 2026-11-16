## 1. Engine

- [x] 1.1 `place_cmds.rs`: `place.open {uri, pane?}` with target-pane rules; registered in `commands.rs`
- [x] 1.2 Tests: SMB place from a Files pane splits into a Network pane (Files pane unchanged, sign-in prompt for the new pane); an existing Network pane is reused; SFTP place opens an SFTP pane; local place navigates in place; bad params are errors

## 2. UI

- [x] 2.1 Places click and context-menu Open, Go → Places dispatch `place.open`

## 3. Wrap-up

- [x] 3.1 CHANGELOG, devlog; test/clippy/fmt/layers/brand-check
- [x] 3.2 Manual: SMB and SFTP bookmark from a Files pane, connected and not (passed, user check 2026-11-16)
