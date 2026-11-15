## Why

Clicking an SMB or SFTP bookmark (for example `\\192.168.0.50`) in a Files pane takes that pane over: after signing in, its folder tree is re-rooted at the server, so the local tree is gone until the user navigates back and the tree is rebuilt. Remote places already have their own pane kinds (Network for SMB, SFTP for SFTP) with a tree made for them; a remote bookmark should open there.

## What Changes

- New command `place.open {uri, pane?}` used by Places clicks, the Places context menu's Open and Go → Places.
- A remote bookmark or place (`smb://`, `sftp://`) opened from a pane that is not already of the matching kind opens in a Network pane (SMB) or SFTP pane (SFTP): an existing one of that kind is reused, otherwise the pane is split and the new half becomes that kind. The original pane keeps its location and tree.
- Sign-in, when needed, happens for the new pane, which shows the bookmark once connected.
- Local places, and remote ones opened from a pane of the matching kind, open in that pane as before.

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `places`: new requirement "Remote places open in their own pane".

## Impact

`fm-engine` (new `place_cmds.rs` with `place.open`, registered in `commands.rs`), `fm-ui` (`pane.rs`, `places.rs`), `fm-app` (`menu.rs` Go → Places). No new dependencies, no prefs change.
