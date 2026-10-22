## Context

Change 1 landed the layered workspace, command registry, job lanes, theme tokens and a placeholder window. `fm-layout` has a split/close/swap pane tree with no per-pane state; `fs-local` is a stub; there is no directory cache, no tree, no listing. Change 2 turns the skeleton into a navigable file manager over **local** directories; remote backends plug into the same pane code in change 4 by implementing `Vfs` (tests already use `fm-testkit`'s in-memory backend to prove the seam).

Constraints: UI thread never blocks (golden rule 4), every behaviour is a command (rule 3), colours only from tokens (rule 5), hostile input is bounded and panic-free (rule 8), no new runtime dependencies.

## Goals / Non-Goals

**Goals:**

- Browse local directories in any pane: tree + listing, selection, sort, navigation history.
- Recursive splits with focus and Tab-swap; layout state lives in `fm-layout` (serializable later).
- All reads and future mutations travel the engine job lanes; the frame never waits on I/O.
- Provably safe on hostile directories (huge, absurd names, special files) with bounded memory.
- Dark and light snapshots for every new widget state.

**Non-Goals:**

- Drag & drop, copy/move/delete/mkdir/rename pipelines (change 3). F5–F8 are not wired yet.
- Real SMB/SFTP backends, watchers, session persistence, packaging (changes 4–5).
- Archive/preview/search/tabs (out of v1).

## Decisions

**D1 — `fs-local` lands now, fully.** Panes need a real backend to demonstrate anything; `testkit` covers engine tests, but tempdir round-trips prove the actual `std::fs` mapping (error variants, `same_device`, vanished-mid-op files). Alternative: keep it a stub until change 3 — rejected, leaves change 2 demo-only.

**D2 — Reads reuse the job lanes as `JobKind::ReadDir`.** One plumbing path: `Session` command → `JobQueue::submit(Lane::Local, ReadDir{uri})` → worker calls `Vfs::read_dir` → `JobEvent` carries entries → `Session::poll()` applies them into `DirCache` and bumps the revision. Alternative: a separate read-request channel — rejected (two event loops, two cancel paths, no benefit).

**D3 — Optimistic navigation.** `nav.open`/`nav.up`/`nav.back`/`nav.forward` validate the target synchronously, commit the pane's `location` and history immediately, then submit the read. The pane shows a *loading* row until the cache has that revision; a failed read shows an *error* row with retry (`nav.refresh`) and the location stays (stale badge). The UI never appears frozen and never blanks.

**D4 — One `DirCache` shared by panes.** Keyed by `VfsUri` (normalized), value `(revision, Arc<[Entry]>)`. Two panes on the same directory share one read; watchers in change 5 only call `invalidate(uri)`. Entries are immutable `Arc` slices so a 200 000-row listing scrolls without cloning.

**D5 — Sort is engine-side, over the cache.** `ListingState.sort: {Name, Size, Mtime, Type} × asc/desc × dirs_first`. Header click re-sorts the cached slice in memory (no re-read); a fresh read is sorted once on apply. UI receives a pre-sorted `Arc<[Entry]>` view index.

**D6 — Tree laziness.** `TreeState { root, expanded: BTreeSet<VfsUri>, scroll }`; children are never stored in the tree — the tree widget reads `DirCache` for expanded nodes and submits a `ReadDir` on first expand. Collapsed nodes hold no children: memory is O(expanded nodes), not O(tree).

**D7 — Selection lives in `ListingState`.** `selection: BTreeSet<String>` (entry names — stable across re-reads at the same location), `focused_row: usize`. Click sets; Shift extends; Ctrl toggles; Ctrl+A selects all; moving the focused row with arrows updates the selection per modifier. Names not indices, so a re-read preserves selection; entries that vanished are pruned on apply.

**D8 — Virtualization is non-negotiable.** Fixed row height from `Tokens`, `ScrollArea::show_rows`; only the visible screenful exists as widgets. 200 000-entry fixture renders; an allocation smoke test asserts bounded widget count.

**D9 — Caps and hostile content.** `MAX_DIR_ENTRIES = 200_000` per read: over the cap the read returns `FmError::Backend("directory too large (>200000 entries)")` — never a silent truncate (the user must know the listing is incomplete if we ever add a truncation mode later, the spec will change first). 4 KiB names, control chars, lone surrogates render via lossy display + tooltip with the full name; special files (fifo/device/socket) render as non-regular entries and cannot be "opened". Non-UTF-8 names are converted with `to_string_lossy` for `Entry.name` — known lossiness for rename in change 3; revisit with `Entry::name_raw` if a test proves a real-world case (open question).

**D10 — Focus model.** Single `focused: PaneId` plus `previous: Option<PaneId>`; click focuses; `pane.focus.next` cycles tree order; `pane.swap` (Tab) swaps the focused pane's subtree with the previous pane's (commander style) and focuses the swap partner. `enabled` predicates keep close/swap disabled on the last pane.

**D11 — Commands and menus.** New `pane_cmds.rs` and `nav_cmds.rs` modules (engine), ids under `pane.*`/`nav.*`/`view.sort.*`; a new top-level `Panes` menu in `fm-app` (small shared-file edit); shortcuts: Tab = `pane.swap`, Backspace = `nav.up`, Alt+Left/Right = back/forward, Ctrl+R = `nav.refresh`, Ctrl+H = toggle sort dirs-first (commander-ish). Menu/shortcut ids stay covered by the brand-check registry cross-check.

**D12 — fm-app render loop.** `render_pane_node(&mut Session, ui, &PaneNode)` recurses: `Leaf` → pane chrome (location bar + tree side panel + listing); `Split` → `egui::Panel` pair with a drag handle writing `ratio` back to `Layout` (pure data, no I/O). Central panel hosts the tree, right side the listing? No — per-pane vertical split: tree on the left (resizable), listing right; both inside the pane's cell. Status bar reads `Session` only.

**D13 — Snapshots.** `egui_kittest` harness in `fm-ui` (dev-dep) builds a harness `Session` from `testkit`'s `MemFs` fixture; states: 1-pane, 2-pane split, tree expanded, rows selected, loading, error — each rendered twice (`Tokens::dark()` / `Tokens::light()`).

## Risks / Trade-offs

- [Optimistic location shows a stale path briefly] → loading badge until the cache revision arrives; error row with retry on failure.
- [Lossy non-UTF-8 names] → documented in D9; `to_string_lossy` only for display keying; revisit in change 3 before rename ships.
- [Huge local dirs still cost an O(n log n) sort] → sort only on read/apply and on header click, never per frame; 200k cap bounds the worst case.
- [Shared `fm-app` menu/shortcut files under parallel agents] → small surgical edits, re-read before edit, registry cross-check keeps ids honest.
- [`show_rows` + nested panels performance] → row virtualization is the guard; if a frame budget regression shows up, the tree side panel collapses to an icon rail (deferred, not planned).

## Open Questions

- Should `nav.open` expose a location bar text input in this change (editable path + Enter)? Currently: display-only bar; typing-to-open lands with tree keyboard nav in this change only if cheap — decide during task 5.x.
- Windows per-file ACLs stay out (`Entry.permissions: Option<u32>` mode bits only, as scaffolded).
