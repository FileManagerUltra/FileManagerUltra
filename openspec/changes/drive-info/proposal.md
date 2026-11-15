## Why

In This PC on Windows the drives read `C:/` and `Data (D:)/`: the listing puts `/` after every folder name, which is not how Windows writes a drive. Selecting a drive shows no drive information: Preview's Volume card only covers drives in the Devices list (removable and user-mounted volumes, never the system drive), and Properties has no drive information at all. Worse, Properties on `C:\` walks the whole drive to count its size, which can take minutes, where Windows simply shows used and free space.

## What Changes

- Drives in This PC (and in Preview's folder table for This PC) are shown without a trailing slash: `C:`, `Data (D:)`.
- The app keeps a cached list of every local volume (name, mount point, filesystem, capacity, free space), refreshed on a background thread every 10 seconds, separate from the Devices list (no new entries appear under Devices).
- Preview shows the Volume information card for a folder that is the root of any local volume (`C:\`, `D:\`, `/`, a mount point), not only for listed devices. Inside a listed device it keeps showing for every folder, as today.
- Properties shows a Drive section for a local volume root: name, filesystem, capacity, used and free space with a used-space bar, and does not walk the drive to count it. For other single items it shows one line: free space of the drive (local) or server (SFTP, SMB) the item is on, when known.

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `preview-pane`: "Volume information in directory previews" also covers the root of any local volume, from the new cached volume list.
- `file-op-commands`: new requirement "Properties shows the drive", including not walking a volume root.
- `file-listing`: new requirement that drive roots in This PC are shown without the folder slash.

## Impact

`fm-vfs` (`VfsUri::is_drive_root`), `fm-engine` (`devices`: local volume list and lookups; `props_cmds`: volume root and free space), `fm-ui` (`listing`, `preview` volume card, `properties`). No new dependencies (`sysinfo` is already used for volume labels). No prefs change.
