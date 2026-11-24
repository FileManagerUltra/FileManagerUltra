## Context

`Entry` carries only kind, size, mtime, permission bits and a hidden flag, and `Vfs` has no way to set times, owner, ACLs or xattrs. The copy pipeline (`fm-ops`) and job lanes (`local`/`smb`/`sftp`) exist and must be reused. Hard constraints from AGENTS.md: no `unsafe`, no new FFI without approval, UI thread never blocks, nothing below L4 names egui, secrets never touch disk, hostile input everywhere.

## Goals / Non-Goals

**Goals:**
- Preserve as many attributes as each backend can express, and say precisely which were not preserved.
- Mirror, update and two-way sync across any `Vfs` pair, with a preview before any write.
- Safe by default: skip on conflict, no deletes without a snapshot, confirmation on mass or permanent deletes.
- Continuous sync while the app is open.

**Non-Goals:**
- Scheduling, headless CLI or background service.
- Auto conflict rules other than `Skip` (the enum is extensible; rules come later).
- Block-level/delta transfer, rename/move detection, bandwidth limits.

## Decisions

**D1 Attribute model in `fm-vfs`.** `Attrs { btime, mtime, atime, mode, uid, gid, owner, group, acl: Option<Vec<u8>>, xattrs: BTreeMap<String, Vec<u8>>, dos_flags }`, all optional, plus `AttrCaps` (bitset) per backend and `AttrSet` selecting which to apply. New trait methods `stat_full(uri) -> Attrs`, `set_attrs(uri, &Attrs, AttrSet) -> AttrReport`, `attr_caps(uri) -> AttrCaps`, all with default impls returning `Unsupported`/empty caps, so existing backends keep compiling. `AttrReport` lists applied attributes and per-attribute failures (permission denied, unsupported). *Alternative:* extend `Entry` with every field. Rejected: listings stay cheap; full attributes are fetched only for files that will be copied or compared.

**D2 Pure-Rust, safe attribute writes.** Local Unix uses `rustix` (already a workspace dep, safe API): `utimensat`, `chownat`, `fsetxattr`/`llistxattr`. Linux ACLs are the `system.posix_acl_*` xattrs, copied as opaque blobs. Times on all platforms use `std::fs::FileTimes` (`set_created` through the platform extension traits on Windows/macOS). Linux cannot set birth time; it is reported as unsupported. Windows ACLs and owner are deferred (D9). SFTP: `setstat` for mode/times/uid/gid. SMB: basic-info set for times and DOS flags; security descriptor only if the `smb` crate exposes it safely, otherwise reported unsupported. *Alternative:* the `xattr`/`filetime` crates; rejected to avoid another dependency where `rustix`/std suffice.

**D3 Apply order.** Content first, then attributes on the file; for directories, attributes after their children (so mtime survives child writes). Ownership before mode (chown clears setuid). The temp-write-then-rename pattern of the copy pipeline is kept; attributes are applied to the temp file before the rename where the backend allows, otherwise right after.

**D4 `fm-sync` at L1** beside `ops`: depends on `fm-vfs` and `fm-ops`, performs no UI work. Pipeline: `scan` (bounded, streaming, cancellable, per side) → `compare` (builds `Vec<Item>` with `Status`) → `plan` (mode + profile → `Vec<Action>`) → `execute` (submits copy/delete/attrs actions through `engine`'s job lanes). Scans of the two sides run concurrently on their own lanes; the walker caps entries and depth and drops names failing `validate_name`.

**D5 Comparison.** Default `size + mtime` with a tolerance (default 2 s, covers FAT/SMB rounding; also an option for ±1 h DST shift). Optional checksum (BLAKE3-class hash, streamed) only when size matches and mtimes differ or the profile forces it. Comparison never reads file content by default.

**D6 Modes.**
- *Mirror*: target becomes identical to source; extras on target are deleted per `DeletePolicy`.
- *Update*: source → target for new/newer; never deletes; newer files on target are left alone and flagged.
- *Two-way*: needs the snapshot. Per path, classify each side as unchanged/new/changed/deleted relative to the snapshot; one-sided change propagates; both changed (or changed vs deleted) is a **conflict**, planned as `Skip` with a warning. No snapshot ⇒ merge: copy missing files both ways, treat differing files as conflicts, delete nothing.

**D7 Snapshot DB.** One file per profile pair id in the state dir (`0600`, dir `0700`), a compact versioned format (serde, atomic temp+rename) with per-path `{kind, size, mtime, hash?}` for both sides. Written only after a run, only for paths that were synced or verified equal, so a crashed run cannot hide a pending change. Corrupt or unknown-version snapshot ⇒ treated as missing (merge, no deletes) plus a notice.

**D8 Deletes.** `DeletePolicy::{Permanent, Trash, Versioning{dir, retention}}`. `Trash` is only valid for local targets (through `ops::trash`). `Versioning` moves the file into `dir/<timestamp>/<relpath>` on the target's backend; retention prunes by age, count or total bytes after a run. Defaults: Trash for local targets, Permanent for remote. The plan carries a `DeleteSummary`; execution requires explicit confirmation when any permanent delete exists or when deletes exceed `max_delete_percent` (default 50) or the source scan is empty while the target is not.

**D9 Windows owner/ACL via `fm-winsec` (user-approved third `unsafe` exception, 2026-11-16).** A new Windows-only L0 crate `fm-winsec` wraps the `windows` crate for security descriptors (`GetNamedSecurityInfoW` / `SetNamedSecurityInfoW`; owner, group, DACL; SACL only when the privilege is held). It follows the `fm-winout` pattern: crate-level `#![deny(unsafe_code)]`, `unsafe` allowed in exactly one module (`sd.rs`), every block with a `SAFETY:` comment, inputs validated in safe code (path rules, blob size cap), output exposed as an opaque, bounded, format-tagged self-relative descriptor (`Attrs.acl = Acl::WindowsSd(Vec<u8>)`) plus resolved owner/group names. The crate compiles to an empty stub on other OSes. `fm-fs-local` calls it behind `cfg(windows)`. Applying a descriptor that needs a privilege the process lacks (e.g. setting an arbitrary owner) is reported per attribute as `PermissionDenied`, never escalated. Descriptors are not translated between Windows SIDs and POSIX uid/gid/ACLs: a Windows-to-Linux pair reports ACL/owner as unsupported for that pair. AGENTS.md, `docs/architecture.md` rule 2 and `xtask/src/layers.rs` are updated in the same change.

**D10 Symlinks.** Policy per profile: `Preserve` (default; recreate link, never follow), `Follow` (copy target; cycle-guarded by device/inode or visited set with depth cap), `Skip`. Link targets that escape the sync root are copied as-is, never dereferenced outside the root.

**D11 Filters.** Include/exclude glob lists evaluated on root-relative, `/`-normalized paths; excludes win; excluded paths are invisible to both compare and delete (they are never deleted from the target by mirror). Glob compilation is size-capped.

**D12 Continuous mode.** A `Watcher` per local side reuses `notify` events (debounced; events only mark the profile dirty, never read inline). Remote sides poll at a profile interval (min 10 s). A dirty profile triggers compare; with `auto_apply` the plan runs only if it contains no conflicts, no permanent deletes and no mass-delete trip, otherwise it stops with a notice for the user. Runs of one profile never overlap; a change during a run schedules exactly one follow-up. Everything stops when the app closes.

**D13 Commands.** `engine/src/sync_cmds.rs`: `sync.open`, `sync.compare`, `sync.run`, `sync.cancel`, `sync.set_action` (row override), `sync.profile.{new,save,delete,rename,duplicate}`, `sync.watch.{start,stop}`. All fuzzed with malformed params. UI and menus dispatch by id only.

**D14 UI.** A dedicated sync view (pane type) in `fm-ui`: top bar (left path, mode, right path, Compare/Sync), centre virtualized grid (left file | action | right file, columns name/size/mtime, filter chips per action, conflict rows highlighted), bottom summary (files, bytes, deletes, conflicts, warnings), side list of profiles. Row models are produced in `engine`; `fm-ui` is data-driven, theme tokens only, snapshot-tested in dark and light.

## Risks / Trade-offs

- [Mirror wipes the target when the source is empty/unmounted] → empty-source and mass-delete guards, mandatory confirmation, default Trash locally.
- [Remote backends cannot preserve everything, giving a false sense of fidelity] → `AttrReport` surfaces unpreserved attributes per file in the preview/log; profile choice of warn/error/ignore.
- [mtime resolution differs (FAT 2 s, SMB 100 ns, SFTP 1 s)] → tolerance setting; compare after normalizing to the coarser side's resolution.
- [Two-way data loss through snapshot drift] → snapshot written only for verified paths; no deletes without a valid snapshot.
- [Large trees (100 000+ files)] → streaming scan with caps, virtualized grid, plan held as compact structs; memory measured in a bench.
- [Watch storms] → debounce, single pending follow-up, no overlap.
- [`rustix` xattr/chown differences across Unix] → capability probing per volume at start; tests skip when the tempdir FS lacks xattrs.

## Migration Plan

Additive. No existing prefs or Vfs implementors break (default trait impls). Profiles and snapshots are new files; removing the feature means deleting them. Prefs gain a `sync` section with `serde(default)`.

## Open Questions

- Windows SID mapping across machines/domains for owner restore: v1 restores by SID and reports failures; name-based mapping is a follow-up.
- SMB security descriptor access through the `smb` crate: confirm during implementation; otherwise unsupported.
