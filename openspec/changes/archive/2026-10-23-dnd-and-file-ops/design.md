## Context

Change 2 left panes browsing through `Vfs` + `DirCache`, with `fm-ops` as typed stubs (`ConflictPolicy`, `OpKind`, `TransferProgress`, `SymlinkPolicy`) and a 3-lane `JobQueue` (`local`/`smb`/`sftp`) already carrying `ReadDir`. Change 3 adds mutation. Constraints: golden rules 1 (no panics), 3 (everything a command), 4 (UI never blocks), 8 (hostile input), 9 (security tested); `ops` stays free of egui/tokio; the UI never calls `ops`.

## Goals / Non-Goals

**Goals:**

- Copy/move/rename/mkdir/delete as cancellable jobs with progress, correct for local↔local now and backend-agnostic for change 4.
- No silent data loss: explicit conflict decisions, verified moves, trash-first delete, undo.
- Drag & drop exactly per architecture §8, resolving to the same engine command the F-keys use.

**Non-Goals:**

- Remote backends, watcher-driven refresh (change 4/5; panes refresh via invalidation after job completion in the meantime).
- Symlink creation via Ctrl+Shift drop (rejected in v1), archives, batch rename, resumable transfers.
- Persisting the undo stack across restarts.

## Decisions

**D1 — `ops` is synchronous and takes a context.** `ops::run(plan, &dyn Vfs resolver, &mut OpCtx)` where `OpCtx` holds a cancel flag (`AtomicBool`), a progress sink (`&mut dyn FnMut(Progress)`) and a conflict callback (`&mut dyn FnMut(Conflict) -> Decision`). The engine worker supplies a callback that parks the job on a channel until the UI decides. Alternative: async pipelines — rejected (only `fs-sftp` owns a runtime; `ops` must stay runtime-free).

**D2 — Plan, then execute.** A planning pass walks sources (bounded by `MAX_DIR_ENTRIES` and a depth cap) producing `Vec<PlanItem { src, dst, kind, size }>` and totals; guards (self/descendant, same-uri, symlink policy) run during planning so a bad drop fails before touching disk. Execution consumes the plan in order (dirs before contents). Alternative: stream-walk while copying — rejected (no totals for progress, guards interleaved with side effects).

**D3 — Conflicts park the worker, never the UI.** On an existing destination under `Ask`, the worker emits `JobEvent::Conflict { job, src, dst, existing }` and blocks on its decision channel (checking the cancel flag on a timeout). `job.resolve_conflict {job, decision, apply_to_all}` sends the answer. Keep-both naming is `name (2).ext`, probed against `Vfs::stat` up to a cap, then error.

**D4 — Move = rename when possible, else copy→verify→delete.** Same authority and `same_device` → `Vfs::rename`. Otherwise copy, verify destination size (and entry kind) equals source, then delete the source; any failure leaves the source intact. Alternative: delete-as-you-copy — rejected (partial failure loses data).

**D5 — Cancel and failures leave no half files.** Files are written to `<name>.fmpart-<id>` in the destination directory and renamed on success; cancel/failure removes the temp. Backends without atomic rename fall back to direct write plus cleanup on failure. Per-item errors are collected (`OpReport { ok, skipped, failed: Vec<(VfsUri, FmError)> }`); the job continues past item errors unless the user cancels, and the report is shown.

**D6 — Trash is a pipeline, not a fallback.** `ops::trash` wraps the `trash` crate for local URIs only; non-local or trash-unavailable returns `FmError::Unsupported` and the UI offers an explicit permanent-delete confirm — never a silent downgrade. Permanent delete walks bottom-up through `Vfs::remove` (the only place a recursive removal exists, behind the confirm).

**D7 — Undo is a bounded stack of inverse records** (`UndoRecord::{Trashed{orig, trash_id}, Moved{from,to}, Renamed{from,to}, Created{uris}}`, cap 100), owned by `Session`. Undo of copy/mkdir moves created items to trash (not hard delete). Undo is best-effort: a record whose target changed returns an actionable error and is dropped. Permanent deletes are not undoable and say so in the confirm.

**D8 — Drag & drop is UI payload + engine resolution.** `fm-ui` sets `DragPayload` (egui `dnd_set_drag_payload`) from selected rows/nodes and, on release, computes `(sources, destination, modifiers)` and dispatches `dnd.drop`. The *engine* resolves the action (same-device rule, guards, no-ops), so tests cover resolution without egui; the UI only draws the badge by asking the engine's pure `resolve_drop()` function. External drops (`raw.dropped_files`) dispatch the same command with `origin = External`.

**D9 — F5/F6 destination is the "other pane".** The most-recently-focused other pane's directory (the Tab-swap partner); with a single pane, F5/F6 open an input dialog for the destination. Alternative: always dialog — rejected (breaks the commander flow).

**D10 — Jobs and refresh.** Mutating jobs run on the lane of the destination (source lane for delete/rename/mkdir); cross-authority copies use the destination lane. On completion the engine invalidates `DirCache` for every touched directory (until the change-5 watcher exists) so panes re-read once.

## Risks / Trade-offs

- [Data loss on move across devices] → copy → verify → delete ordering + fault-injection tests (source untouched on any failure).
- [Cancel leaves junk] → temp-name writes with cleanup; test cancels at every progress tick.
- [Conflict dialog deadlock if UI never answers] → the parked worker polls the cancel flag; closing the dialog = `Cancel`; job removal drops the channel, which resolves as cancel.
- [Huge selections] → plan size capped (items and depth); over-cap returns an actionable error before any write.
- [Time-of-check races on guards] → guards are best-effort preflight; execution re-checks `AlreadyExists`/`NotFound` and routes through the same conflict/error paths.
- [`trash` semantics differ per OS] → wrapped behind one function; tests run where a trash exists and skip otherwise (like the GPU-less snapshot skip).
- [Undo expectation mismatch] → undo scope is documented in the spec; permanent delete confirm states it is irreversible.

## Migration Plan

Additive: stubs in `fm-ops` are replaced with the same public names where possible (`copy`/`move_`/`delete`/`mkdir`/`rename` become thin wrappers over `run`). The `stubs_are_unsupported_not_panics` test is replaced by behaviour tests. No data migration.

## Open Questions

- Trash restore for undo: use the `trash` crate's `os_limited` API (not on macOS) or record-and-move-back where the platform exposes the trash path? Decide in task 5.2 after a spike; macOS may ship with "undo delete" disabled and documented.
