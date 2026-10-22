# Tasks — multi-pane-ui

## 1. Layout state model (`fm-layout`)

- [ ] 1.1 `TreeState { root, expanded: BTreeSet<VfsUri>, scroll }` with expand/collapse/clear helpers; invariant: root always expanded
- [ ] 1.2 `ListingState { sort: SortKey+direction+dirs_first, selection: BTreeSet<String>, focused_row, scroll }` with click/shift/ctrl/all selection helpers and prune-on-re-read
- [ ] 1.3 `History { back: Vec<VfsUri>, forward: Vec<VfsUri> }` — push clears forward, back/forward return `Option`
- [ ] 1.4 Extend `PaneState` to hold tree/listing/history; tests: split preserves sibling state, close passes state to nobody, swap exchanges whole states, selection survives a re-read with the same names

## 2. Local backend (`fs-local`)

- [ ] 2.1 Implement `stat`/`read_dir` with entry caps (`MAX_DIR_ENTRIES`), kind detection (dir/file/symlink/special), mtime/size, lossy name conversion; map `io::ErrorKind` → `FmError` variants
- [ ] 2.2 Implement `open_read`/`open_write` (create/truncate/append per `WriteOpts`), `create_dir`, `remove` (file + empty dir; recursive refused via `NotEmpty` semantics for dirs), `rename`, `same_device`, `space`
- [ ] 2.3 Tempdir round-trip suite: full trait walk, typed errors (NotFound/PermissionDenied/NotADirectory/NotEmpty/AlreadyExists), vanished-mid-op, `..`/NUL inputs rejected at the URI layer before any call
- [ ] 2.4 Hostile-fixture test: >cap directory errors actionably; fifo/socket entries report `Other` kind and are not opened as files

## 3. Engine: cache, jobs, poll (`fm-engine`)

- [ ] 3.1 `DirCache`: normalized-uri → `(revision, Arc<[Entry]>, Option<FmError>)`; `get/insert/invalidate/clear`; revision strictly monotonic per uri
- [ ] 3.2 Add `JobKind::ReadDir { uri }`; local lane worker calls `Vfs::read_dir`, sorts per default sort, emits `JobEvent` with entries or error; `Session::poll` applies into `DirCache` and bumps a `layout_dirty`/`cache_changed` flag for repaint
- [ ] 3.3 `Session` gains a `VfsRegistry` hook (scheme → `Arc<dyn Vfs>`; local registered by default, testkit MemFs injectable in tests); `submit_read(uri)` picks the lane from the scheme
- [ ] 3.4 Tests: one read serves two panes (dedup in-flight), failed read surfaces the error in the cache, refresh invalidates then re-reads, poll never blocks (slow MemFs returns before entries arrive)

## 4. Engine commands (`fm-engine`)

- [ ] 4.1 `pane_cmds.rs`: `pane.split.horizontal`, `pane.split.vertical`, `pane.close`, `pane.swap`, `pane.focus.next` — `enabled` predicates (close needs >1 pane; swap needs a previous pane); layout mutations only
- [ ] 4.2 `nav_cmds.rs`: `nav.open` (params: `uri` string; validates, commits location+history, submits read), `nav.up`, `nav.back`, `nav.forward`, `nav.refresh`; Backspace/Alt+Left/Alt+Right/Ctrl+R shortcut metadata
- [ ] 4.3 `view.sort.*` commands (`name`, `size`, `mtime`, `type`, `dirs_first` toggle) mutating the focused pane's `ListingState` and re-sorting cached entries in place
- [ ] 4.4 Register all ids in `commands.rs`; extend the command fuzz suite (empty/unknown/wrong-typed/oversized params → `Ok`/`Err`, never panic); `enabled`-false commands return `Ok` no-ops through dispatch

## 5. UI widgets (`fm-ui`)

- [ ] 5.1 `tree.rs`: lazy `TreeWidget` reading `DirCache` through `&Session`; expand on click/Right, collapse Left, Enter opens; inline loading/error affordances; 4 KiB-name rows ellipsize with full-name tooltip
- [ ] 5.2 `listing.rs`: virtualized `ListingWidget` (`ScrollArea::show_rows`, fixed row height from `Tokens`); `..` row; columns name/size/mtime/type; header-click sort dispatch; selection + keyboard navigation per spec; loading/empty/error states
- [ ] 5.3 `pane.rs`: pane chrome — location bar, tree/listing split (tree width adjustable), focus ring on the focused pane, stale badge hook for failed reads
- [ ] 5.4 All colours/metrics from `Tokens`; `cargo xtask brand-check` stays green (no `Color32::` literals outside `theme.rs`)

## 6. App wiring (`apps/fm-app`)

- [ ] 6.1 Recursive `render_pane_node`: `Leaf` → pane chrome; `Split` → two panels + drag handle writing ratio to `Layout` (clamped); central area replaced
- [ ] 6.2 Focus model: click focuses; track previous focused pane; `Panes` menu added to the menu catalogue wired to the new command ids; shortcuts Tab/Backspace/Alt+Left/Alt+Right/Ctrl+R registered and routed through dispatch
- [ ] 6.3 Status bar: focused pane location, item count, selection count, current theme — all from `Session`, updated on the same frame as cache application
- [ ] 6.4 Session bootstrap: default layout + `LocalFs` registered before the first frame; startup shows one empty pane (loading → listing) without blocking the first paint

## 7. Security & hostile fixtures

- [ ] 7.1 `testkit`: builder for hostile trees (200 001 entries, 4 KiB names, control chars, non-UTF-8 bytes, fifo/socket where the platform allows)
- [ ] 7.2 End-to-end hostile test: hostile tree through `LocalFs`/`MemFs` → `DirCache` → tree+listing render in a harness — no panic, bounded rows, capped read errors surfaced as the pane error state
- [ ] 7.3 Property/edge tests: selection helpers on adversarial name sets; history on pathological push sequences; sort on empty/singleton/all-duplicates key sets

## 8. Snapshots (`egui_kittest`)

- [ ] 8.1 Dev-deps + harness: `egui_kittest` in `fm-ui`, harness `Session` from a `MemFs` fixture, tokens applied per theme
- [ ] 8.2 Snapshot states × dark/light: 1-pane empty, 1-pane populated+selected rows, tree expanded, 2-pane split, loading, error-with-retry
- [ ] 8.3 Snapshot update documented in `docs/development.md`

## 9. Gate, docs, wrap-up

- [ ] 9.1 Whole gate green: `cargo test --workspace`, clippy `-D warnings`, `fmt --check`, `cargo xtask layers`, `brand-check`, `cargo xtask ci`
- [ ] 9.2 Manual smoke on a real directory (`cargo run -p fm-app`, browse `/usr`, split, sort, select, back/forward) recorded in the devlog
- [ ] 9.3 Docs sync: `docs/architecture.md` §6/§7/§9 updated to the landed shape; roadmap marks change 2 done on archive
- [ ] 9.4 `log/devlog.md` entry with numbers (tests, rows virtualized, fixture sizes); user asked to commit + archive
