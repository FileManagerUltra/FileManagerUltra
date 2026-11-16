## 1. Drive names

- [x] 1.1 `VfsUri::is_drive_root` (+ OS-parameterised check) with tests
- [x] 1.2 Listing and Preview's folder table: no slash after a drive root

## 2. Volume cache

- [x] 2.1 `Volume` type, `Device::volume()`, `Devices::{local, volume_root, volume_for}`
- [x] 2.2 `platform::local_volumes()` (sysinfo, all OSes; Windows names `Label (C:)` / `Local Disk (C:)`)
- [x] 2.3 Own single-flight scan thread, 10 s interval, applied in `poll`; tests with injected volumes

## 3. Preview and Properties

- [x] 3.1 Preview: volume card for a volume root (any local volume) as well as inside listed devices; `volume_card` takes `Volume` + removal hint
- [x] 3.2 Properties: a volume root is not walked; Drive section with name, filesystem, capacity, used, free and bar
- [x] 3.3 Properties: free space of the containing drive/server for other single items (`vfs.space` on the props thread)
- [x] 3.4 Tests: volume root lookup, Properties of a volume root does not walk, free space on an in-memory backend

## 4. Wrap-up

- [x] 4.1 CHANGELOG, devlog; test/clippy/fmt/layers/brand-check; snapshots if touched
- [x] 4.2 Manual (Windows): This PC names, Preview of C:, Properties of C: and of a file on a share (passed, user check 2026-11-16)
