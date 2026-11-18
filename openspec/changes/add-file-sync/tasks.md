## 1. Attribute layer (L0)

- [x] 1.1 Add `Attrs`, `AttrCaps`, `AttrSet`, `AttrReport` to `fm-vfs` with caps/bounds on xattr names, counts and blob sizes; unit tests incl. hostile values
- [x] 1.2 Add `stat_full`, `set_attrs`, `attr_caps` to `Vfs` with default `Unsupported` impls
- [x] 1.3 Implement in `fm-fs-local`: std `FileTimes` (+ platform created-time ext), `rustix` chown/xattr/ACL blobs, symlink-safe; tempdir round-trip tests (skip when FS lacks xattrs)
- [x] 1.3a Create `fm-winsec` (L0, Windows-only, `deny(unsafe_code)` + single allowed `sd.rs`, `SAFETY:` comments, bounded blobs, stub on other OSes); register in `xtask/src/layers.rs`; wire into `fm-fs-local` under `cfg(windows)`; Windows-gated round-trip tests (owner, DACL, inheritance flags)
- [x] 1.3b Update AGENTS.md rule 2 and the dependency notes, `docs/architecture.md` rule 2, and the `unsafe` allow-list check (if any in `xtask`) for the `fm-winsec` exception
- [x] 1.4 Implement in `fm-fs-sftp` (setstat mode/times/uid/gid, caps) with in-process server tests
- [x] 1.5 Implement in `fm-fs-smb` (times, DOS flags; SD only if safe in `smb` crate) with Docker-gated tests
- [x] 1.6 Implement in `fm-testkit` in-memory Vfs so higher layers can test attributes deterministically

## 2. Copy pipeline (L1)

- [x] 2.1 Teach `fm-ops` copy to apply selected attribute groups (content, then owner, mode, times; dirs after children) and return `AttrReport`s
- [x] 2.2 Tests: directory mtime preserved, partial failure keeps content, per-attribute selection

## 3. `fm-sync` crate (L1)

- [x] 3.1 Create `crates/sync`, register in `xtask/src/layers.rs`, workspace manifests, `cargo xtask layers` green
- [x] 3.2 Profile model (mode, compare, filters, symlink/attr/delete policy, `ConflictRule::Skip`, watch settings) with serde defaults and validation (overlap, remote Trash, URI limits, no secrets)
- [x] 3.3 Bounded streaming scanner per side (entry/depth caps, `validate_name`, cancellation, glob filters with size cap, symlink policy with cycle guard)
- [x] 3.4 Comparer: size+mtime with tolerance, optional streamed checksum with `blake3` (pure, default features off, exact pin; add notice if licence needs it), directory/attribute comparison
- [x] 3.5 Snapshot store: versioned format, atomic write, owner-only perms, corrupt/unknown version treated as missing; unit tests
- [x] 3.6 Planner for mirror, update, two-way (new/changed/deleted classification, conflicts as Skip, merge without snapshot); property tests on the in-memory Vfs
- [x] 3.7 Deletion safeguards (permanent, `max_delete_percent`, empty source) producing a confirmation-required result
- [x] 3.8 Executor: copy/delete/attrs actions through the copy pipeline, `DeletePolicy` (Permanent, Trash via `ops::trash`, Versioning with retention pruning), snapshot update only for verified paths, cancel, throttled progress
- [x] 3.9 Tests: each mode, hostile tree (`hostile_tree()`), crash-mid-run snapshot, versioning retention, traversal names, empty source

## 4. Engine integration (L3)

- [x] 4.1 `engine/src/sync_cmds.rs`: `sync.open/compare/run/cancel/set_action`, `sync.profile.*`, `sync.watch.*` with ids, labels, menu paths, params docs, `enabled`; extend `specs()` in `commands.rs`
- [x] 4.2 Run scans/executions on the scheme job lanes; apply results only in `Session::poll()`; plan/row model exposed for the UI
- [x] 4.3 Prefs: persist profiles with `serde(default)`, owner-only files, no secrets; credentials via the connection store
- [x] 4.4 Watch manager: debounced `notify` for local sides, polling (min 10 s) for remote, dirty flag only, single follow-up, no overlap, conservative auto-apply
- [x] 4.5 Malformed-params fuzz tests for every new command; graceful-failure tests

## 5. UI (L4/L5)

- [x] 5.1 Theme tokens for action markers/conflict rows in `Tokens::dark()` and `Tokens::light()`
- [x] 5.2 Sync view in `fm-ui`: top bar, virtualized grid, filter chips, per-row action override, summary bar, profile list, confirmation dialog
- [x] 5.3 `egui_kittest` snapshots in dark and light; 100 000-row virtualization test
- [x] 5.4 Wire menu entries and shortcuts in `fm-app`; verify dispatch by id only

## 6. Docs and finish

- [x] 6.1 Update `docs/architecture.md` (crate map, attribute layer, sync flow), `docs/roadmap.md`, `docs/contributing.md` if needed
- [x] 6.2 `CHANGELOG.md` `[Unreleased]` user-facing lines; `log/devlog.md` entry
- [x] 6.3 Run `cargo test` for touched crates, `cargo clippy --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`

## 7. Guided and advanced workflows

- [x] 7.1 Engine: `SyncState.view`/`step`, commands `sync.view` and `sync.wizard.go` (review compares, keep-in-sync arms), fuzz and bad-param tests
- [x] 7.2 UI: workflow switch, seven-step wizard with option cards, review with start/back/save/compare-again and per-button explanations, hints in the advanced toolbar
- [x] 7.3 Interaction test through the wizard, snapshots (goal, review) in dark and light

