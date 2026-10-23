## Why

Change 2 made the app a file manager you can *look at*; it still cannot change anything — `fm-ops` is a set of `Unsupported` stubs and F5–F8 are unwired. Change 3 makes it a file manager you can *work with*: copy, move, rename, mkdir and delete as cancellable background jobs, with conflict decisions, trash + undo, and the cross-pane drag & drop contract from `docs/architecture.md` §8.

## What Changes

- **`fm-ops` pipelines implemented** over the `Vfs` trait (so they already work local↔local and, later, local↔remote): `copy`, `move` (rename when same authority/device, else copy + verified delete), `delete` (via trash), `mkdir`, `rename`; recursive walking with a planning pass (item/byte totals), streaming `open_read`/`open_write` with bounded buffers, cooperative cancel and typed per-item errors.
- **Guards enforced in `ops`, not the UI**: never copy/move a directory into itself or a descendant, same-uri copy is a silent no-op, no overwrite without a decision, the `SymlinkPolicy` from the `security` capability (symlinks copied as links, never followed out of the source tree).
- **Conflict resolution**: `ConflictPolicy::Ask` becomes a job state — the worker parks on a conflict, the engine surfaces it, the UI shows a dialog (Overwrite / Keep both / Skip / Cancel, each with "apply to all"), the decision flows back by command.
- **Job progress and cancel**: `JobEvent` gains `Progress { done, total, current }`, `Conflict`, `Finished`, `Failed`; a jobs panel/status-bar indicator shows running jobs with progress bars and a cancel button; `job.cancel` command.
- **Trash + undo**: delete goes through the `trash` crate (`ops::trash`, never `remove_dir_all`); a bounded undo stack records trash/move/rename/mkdir/copy-created inverses and `edit.undo` replays them. Permanent delete (Shift+Delete) needs an explicit confirm.
- **Commands** (all dispatched by id): `file.copy` (F5), `file.move` (F6), `file.mkdir` (F7), `file.delete` (F8 / Delete), `file.delete_permanent` (Shift+Delete), `file.rename` (F2), `edit.undo` (Ctrl+Z), `job.cancel`, `job.resolve_conflict`, and the drop-resolution command `dnd.drop` (params: sources, destination, action). F5/F6 use the *other* pane's directory as destination.
- **Drag & drop** (§8): `DragPayload` set from listing rows and tree nodes, drop targets (tree node/background, listing row/background, `..` row), modifier resolution (none = move on same device else copy, Ctrl = copy, Shift = move, Esc = cancel), hover outline + badge (`Copy 12 items` / `Move 3 items`), `NoDrop` cursor on invalid targets; external OS file drops land in the hovered pane via the same command.
- **Dialogs/widgets in `fm-ui`**: conflict dialog, confirm-delete dialog, mkdir/rename input dialogs, jobs panel — tokens only, dark + light snapshots.

## Capabilities

### New Capabilities

- `file-operations`: the `ops` pipelines (copy/move/delete/mkdir/rename), planning, streaming, guards.
- `conflict-resolution`: conflict detection, the park-and-decide protocol, apply-to-all, keep-both naming.
- `job-progress`: job events, progress/ETA data, cancel, jobs panel.
- `trash-and-undo`: trash-based delete, permanent-delete confirm, bounded undo stack.
- `drag-and-drop`: payload, drop targets, modifier resolution, feedback, external drops.
- `file-op-commands`: the file/edit/job commands, their shortcuts, enabled predicates and param validation.

### Modified Capabilities

- `security`: add containment/overwrite requirements for ops and drop payloads (see delta).

## Impact

- **Code:** `crates/ops` (pipelines, trash, undo records), `crates/engine` (`file_cmds.rs`, `job_cmds.rs`, `dnd_cmds.rs`, job events, undo stack, conflict parking), `crates/ui` (dnd helpers, dialogs, jobs panel), `apps/fm-app` (shortcuts F2/F5–F8/Delete/Ctrl+Z, menus, external drop wiring, `with_drag_and_drop(true)`), `crates/testkit` (fault-injecting `Vfs`: slow, failing, vanishing files).
- **Dependencies:** `trash` (runtime, `fm-ops` only — already named in AGENTS.md §7). No tokio or egui in `ops`.
- **Docs:** `docs/architecture.md` §7/§8 refined to the landed shape; roadmap marks change 3 done on archive.
- **Risks:** data loss is the hazard of this change — every destructive path needs a failing-test proof (no overwrite without decision, move never deletes before the copy is verified, cancel leaves no half-written destination, trash failure never falls back to permanent delete silently). Blocking the UI thread on I/O stays forbidden; all work runs on job lanes.
