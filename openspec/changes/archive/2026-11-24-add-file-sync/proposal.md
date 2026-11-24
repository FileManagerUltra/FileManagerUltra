## Why

Users keep folders in step by hand (copy, compare, merge) and lose timestamps, ownership and permissions on the way. A built-in sync that works across local, SMB and SFTP, preserves as many file attributes as each side supports, and shows a preview before touching anything removes that chore and the risk that comes with it.

## What Changes

- Extend the `Vfs` contract with full attribute read/write (`stat_full`, `set_attrs`) and a per-backend capability set: creation/modification/access time, mode, owner/group, ACLs, xattrs, DOS flags. Unsupported attributes degrade to per-attribute warnings, never to a failed copy (unless the profile says so).
- New `fm-sync` crate (L1): folder scanner, comparer (size + mtime with tolerance, optional checksum), include/exclude globs, symlink policy, action planner for three modes (**mirror**, **update**, **two-way**), executor on top of the existing copy pipeline.
- Two-way change detection through a per-pair snapshot database (last synced state) in the app state dir, owner-only. Without a snapshot the first run is a merge and never deletes.
- Conflicts default to **skip** (shown as a warning until the user picks a direction); the profile model carries a `ConflictRule` enum so auto-rules can be added later without migrating profiles.
- Per-profile `DeletePolicy`: `Permanent`, `Trash` (local only), `Versioning(folder, retention)`. Default is Trash on local targets and Permanent (with a mandatory confirmation) on SMB/SFTP. Mirror asks for confirmation when more than a configurable share of the target would be deleted.
- Saved sync profiles (pair, mode, filters, attribute policy, delete policy, comparison method) persisted in prefs without secrets.
- FreeFileSync-style but simpler UI: left/right folder pickers, mode selector, comparison grid (left file, action arrow, right file) with per-row action override and filter by action, summary bar (counts, bytes), Compare and Synchronize buttons, profile list.
- Continuous sync while the app is open: local sides use `notify` (debounced), remote sides poll on an interval; changes trigger a compare and, when auto-apply is on, a run. No scheduler and no headless mode.
- Everything is a command (`sync.*`), dispatched by id from menus, shortcuts and the UI.

## Capabilities

### New Capabilities
- `file-attributes`: the attribute model, capability reporting and best-effort preservation rules across backends.
- `file-sync`: scan, compare, plan and execute for mirror/update/two-way, snapshots, conflicts, filters, symlinks, delete policies and safety checks.
- `sync-profiles`: saved profiles and the continuous-watch behaviour while the app is open.
- `sync-ui`: comparison grid, summary, action override and profile management.

### Modified Capabilities
- `local-backend`: read/write the full attribute set (times, owner/group, ACLs, xattrs, flags), including Windows owner/ACL through `fm-winsec`.
- `sftp-backend`: read/write mode, times, uid/gid; report capabilities.
- `smb-backend`: read/write times and DOS flags (and security descriptor where the crate allows); report capabilities.
- `security`: snapshot DB and profile files are owner-only; no credentials in profiles, snapshots or sync logs; destructive-action confirmation rules.

## Impact

- New crate `crates/sync` (`fm-sync`, L1) registered in `xtask/src/layers.rs`; new `engine/src/sync_cmds.rs`; new `fm-ui` sync view and theme tokens (dark and light).
- `fm-vfs` trait and `Entry` gain attribute types; `fm-fs-local`, `fm-fs-smb`, `fm-fs-sftp`, `fm-testkit` implement them.
- `fm-ops` copy pipeline learns to apply attributes after write and on directories after their contents.
- New Windows-only crate `fm-winsec` (L0, `windows` crate): user-approved third `unsafe` exception for owner/group/ACL, confined to one module; AGENTS.md and architecture docs updated. Unix uses safe `rustix`.
- New dependency `blake3` (pure build, default features off) for checksum comparison.
- Docs: `docs/architecture.md`, `docs/roadmap.md`, `CHANGELOG.md`.
