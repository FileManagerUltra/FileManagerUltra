# Tasks — dnd-and-file-ops

## 1. Test support and ops core (`fm-testkit`, `fm-ops`)

- [x] 1.1 `fm_testkit`: fault-injecting `Vfs` wrapper (slow reads, fail-after-N-bytes, vanishing file, rename-unsupported) with tests
- [x] 1.2 `ops::plan`: walk sources into `Plan { items, total_items, total_bytes }` with entry/depth/item caps and the containment/same-uri guards; symlinks planned as `Symlink` items and reported unsupported (never followed — `Vfs` cannot create links); tests incl. into-self, descendant, over-cap, symlink escape
- [x] 1.3 `OpCtx` (cancel flag, progress sink, conflict callback) and `OpReport`; temp-name write (`.fmpart-<id>`) + rename-on-success helper with cleanup
- [x] 1.4 `ops::copy` streaming pipeline; tests: tree copy, cross-backend (MemFs → fs-local), vanished source, cancel at every tick leaves no temp
- [x] 1.5 `ops::move_`: rename fast path, copy→verify→delete fallback; fault tests prove the source survives every failure
- [x] 1.6 `ops::mkdir`/`ops::rename` with name validation; tests for invalid names and existing targets

## 2. Conflicts (`fm-ops`)

- [x] 2.1 Conflict detection with `Ask`/`Overwrite`/`KeepBoth`/`Skip`, apply-to-all state, keep-both naming probe + cap
- [x] 2.2 Overwrite via temp-then-rename; tests: no overwrite without decision, apply-to-all, Cancel ends job with cleanup

## 3. Trash and undo (`fm-ops`)

- [x] 3.1 Add `trash` dependency; `ops::trash` (local only, `Unsupported` elsewhere, never falls back to permanent delete); skip-if-no-trash test
- [x] 3.2 `ops::delete_permanent` bottom-up through `Vfs::remove`
- [x] 3.3 `UndoRecord` + bounded `UndoStack` (cap 100) with inverse application; tests: undo rename/move/mkdir/copy, stale record, permanent delete pushes nothing
- [x] 3.4 Resolve the trash-restore open question (design) with a spike; document the platform outcome in `docs/development.md`

## 4. Engine jobs and commands (`fm-engine`)

- [x] 4.1 Extend `JobKind`/`JobEvent` (Progress, Conflict, Finished, Failed) and a per-job handle (cancel flag, decision channel); jobs table exposed via `Session`
- [x] 4.2 `file_cmds.rs`: `file.copy`, `file.move`, `file.mkdir`, `file.delete`, `file.delete_permanent`, `file.rename`, `edit.undo` with enabled predicates and other-pane destination rule
- [x] 4.3 `job_cmds.rs`: `job.cancel`, `job.resolve_conflict`; parked-job cancel/drop-channel tests
- [x] 4.4 `dnd_cmds.rs`: pure `resolve_drop()` (modifier table, same-device, guards) + `dnd.drop` command; table-driven tests of the §8 matrix
- [x] 4.5 Completion invalidates `DirCache` for touched directories; test: second pane re-reads once
- [x] 4.6 Register ids in `commands.rs`; extend the fuzz suite (empty/unknown/wrong-typed/oversized params → `Err`, never panic); poll-never-blocks test with a slow backend

## 5. UI (`fm-ui`)

- [x] 5.1 DnD: payload from listing rows and tree nodes, drop-target detection, hover outline + badge from `resolve_drop()`, `NoDrop` cursor
- [x] 5.2 Dialogs: conflict (4 choices + apply-to-all), confirm-permanent-delete, mkdir/rename input with live validation
- [x] 5.3 Jobs panel + status-bar indicator (progress bars, cancel, report with per-item errors, dismiss)
- [x] 5.4 Tokens only (no `Color32::` outside `theme.rs`); dark + light `egui_kittest` snapshots for each dialog, jobs panel, drop highlight/badge

## 6. App wiring (`apps/fm-app`)

- [x] 6.1 Shortcuts F2/F5/F6/F7/F8/Delete/Shift+Delete/Ctrl+Z and File/Edit menu entries with shortcut text; brand-check id cross-check stays green
- [x] 6.2 `with_drag_and_drop(true)`; external `dropped_files` → `dnd.drop` (external origin) into the hovered pane; hostile-path test
- [x] 6.3 Render dialogs/jobs panel each frame; dispatch only through `Session::dispatch`

## 7. Security, docs, gate

- [x] 7.1 Security tests from the delta: traversal names, cancelled write leaves nothing, no silent trash downgrade, redacted errors
- [x] 7.2 `docs/architecture.md` §7/§8 synced to the landed shape; `docs/development.md` manual DnD/ops smoke checklist; roadmap status
- [ ] 7.3 Whole `cargo xtask ci` green; manual smoke on a real window (user); `log/devlog.md` entry with numbers; commit + `/opsx-archive` on user go-ahead

## 8. Follow-ups (post-group 7)

- [x] 8.1 `New File` (`file.newfile`, Shift+F7): empty file via `Vfs::open_write`, never overwrites; undoable
- [x] 8.2 Symlinks: `Vfs::create_symlink` (local unix/windows, `MemFs`), copy recreates links, `retarget_links` preference + `prefs.links.retarget` command
- [x] 8.3 Preferences dialog (`app.preferences`, Ctrl+, / Cmd+,): theme and link re-targeting; dark + light snapshot
- [x] 8.4 Editable location bar (`nav.goto` Ctrl+L / click path, `nav.goto.commit`, `nav.goto.cancel`): directory completion from `DirCache` (non-blocking), session history (100), Tab completes, Up/Down select, Esc cancels; Go menu
- [x] 8.5 Tree follows the pane (expand + focus + scroll), arrow-key navigation fixes (Right/Left/Enter), link-target toggle (`view.links.toggle`) and link icon, X11 forcing for OS drops on Wayland
