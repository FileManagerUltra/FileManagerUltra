## Context

The listing appends `/` to every folder name (`listing.rs`), and Preview's folder table does the same; This PC's entries are drive roots named `C:` with an optional label `Data (D:)`. Volume details exist only for the Devices list (`devices::Device`: name, mount, filesystem, total, free), which the platform scan fills with removable and user-mounted volumes; on Windows it skips the system drive. Preview's `volume_card` reads that cached list. Properties walks every item on its own thread; `Vfs::space(uri)` reports total and available space on every backend (local via `sysinfo`, SFTP via statvfs, SMB via the share's size info).

## Goals / Non-Goals

**Goals:**
- Drive names without a slash.
- Volume details for every local volume root in Preview and Properties, from a cache (never I/O on the UI thread).
- Free space for any item in Properties, on every backend.

**Non-Goals:**
- New Devices entries (the system drive stays out of Devices).
- A volume card in Preview for remote folders.
- Changing the Properties walk for remote share roots.

## Decisions

1. **`VfsUri::is_drive_root()`**: Windows only, a local `file` path of exactly `/X:` (the children of `file:///`). The pure check takes the OS as a parameter for tests, like `display_form`.
2. **Local volume cache in `Devices`**: `local: Vec<Volume>` with its own single-flight thread (`volumes-scan`) and a 10 s interval, started from `devices_tick` and applied in `poll` like the device scan, so a slow volume query never delays Devices discovery and vice versa. `platform::local_volumes()` uses `sysinfo::Disks` with storage refresh on every OS; on Windows a volume is named `Label (C:)` or `Local Disk (C:)` as Explorer does. Capped at 256 entries.
3. **`Volume` type** shared by devices and local volumes (`name, mount, filesystem, total, free`); `Device::volume()` converts. Lookups: `Devices::volume_root(uri)` (a device or local volume whose mount equals `uri`) and `Devices::volume_for(uri)` (closest containing device, else closest local volume). Preview uses the closest device for any descendant (unchanged) and otherwise `volume_root`.
4. **Properties**: at start, a single item that is a cached `volume_root` is not walked; the dialog shows the Drive section and Size = used. Otherwise the props thread asks `vfs.space(item)` once, before walking, and stores it in `PropsInfo::space`; the dialog shows "Free space: X free of Y" (plus "on <volume>" when the local cache knows it).
5. **`volume_card`** takes a `Volume` plus an optional removal hint, so devices keep their unsupported-removal explanation.

## Risks / Trade-offs

- [`sysinfo` on Windows can stall on a dead network drive] → It runs on its own thread with single-flight; a stall only delays the next volume refresh.
- [Right after start the cache is empty] → Preview shows the card once the first scan lands (within moments); Properties on a drive opened in that window falls back to walking, as before.
