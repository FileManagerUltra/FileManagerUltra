## Why

Find Files works in local folders and on SFTP servers but refuses SMB shares ("Find Files works in local folders and on SFTP servers"), and Properties shows "Folder contents are not counted on this kind of location" for a folder on a share. Both come from an early rule that SMB work stays on the SMB job lane. The reason for the lanes is that a hung share must not block other work (`docs/architecture.md` §7), and a walk on its own thread keeps that: a hung share then blocks only that walk. The SMB backend opens and closes a handle per operation and already serves two lane workers at once, so one more reader is safe. Users browse NAS shares through the Network folder and expect the same tools there as on SFTP.

## What Changes

- Find Files searches SMB shares (`smb://host/share/...`, shown as `\\host\share\...` on Windows) with the same single-threaded walker and limits as SFTP: no symlinks, 10 000 results, depth 64, binary files skipped for content, at most 64 MiB read per file, Stop and closing the window cancel.
- Properties counts folder size and items on SMB shares with the same background walk and caps as local and SFTP folders.
- Locations with nothing to search get a message that says what to do instead of the generic refusal: the Network and Servers folders ("Open a share or server to search it") and an SMB server root such as `\\nas` ("Open a share to search it").
- The rule in `AGENTS.md` §7 and `docs/architecture.md` §7 is narrowed: directory reads and file jobs stay on the `smb` lane; bounded single-thread walks (Find Files, Properties sizes) may use the session from their own thread.

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `file-search`: "Search is bounded and safe" now includes SMB shares and names the locations that cannot be searched.
- `file-op-commands`: new requirement that Properties counts folder contents on every backend that has folders (local, SFTP, SMB shares).

## Impact

`fm-engine` (`search_cmds::start`, `props_cmds::start`), `AGENTS.md`, `docs/architecture.md`. No new dependencies, no UI layout change, no prefs change. SMB timeouts already bound each blocked call.
