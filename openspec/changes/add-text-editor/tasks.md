## 1. Spike and dependencies

- [x] 1.1 Spike: add `tree-sitter` plus the grammar crates behind cargo features in a scratch crate; measure binary-size delta, parse/highlight time on 1 MB / 32 MB source files, and CI build time on Linux, macOS and Windows; record results and the chosen `syntax_max_bytes` and parse time budget in `design.md`
- [x] 1.2 Add `ropey`, `tree-sitter` and the chosen grammars to the workspace; update `deny.toml`, `THIRD-PARTY-NOTICES.md`, `fm_preview::notices`; run `cargo deny check` (all MIT, permissive: no notice entries needed; `cargo deny check` passes unchanged; `tree-sitter-sql` dropped — it targets tree-sitter 0.19 and cannot be used, SQL uses a small built-in lexer)
- [x] 1.3 Create `crates/editor` (`fm-editor`, L1); register in `xtask/src/layers.rs`; `cargo xtask layers` passes

## 2. Buffer core (`fm-editor`)

- [x] 2.1 `Buffer` on `ropey` with bounds-checked `(line, col)` API returning `Result`, line index and chunked append for streaming load
- [x] 2.2 Coalescing undo/redo log with a configurable step limit (default 1000, 10–100000) and a 64 MiB memory cap
- [x] 2.3 Encoding (UTF-8/BOM, UTF-16, Latin-1) and EOL detection and preserving save serialization; binary sniffing
- [x] 2.4 Find/replace engine (literal, whole word, case, regex with size limit and time budget, cancellable)
- [x] 2.5 Edit helpers: auto-indent, indent/outdent, duplicate/delete/move line, toggle comment, bracket matching
- [x] 2.6 Tests: round-trips, CRLF/BOM/UTF-16, 1M-line buffer ops, malformed positions return `Err`, proptest-style random edit/undo sequences

## 3. Syntax colouring

- [x] 3.1 Language detection (extension, file name, shebang) and feature-gated grammar registry
- [x] 3.2 Incremental parser per buffer on a bounded worker pool with cancel, time budget and size threshold
- [x] 3.3 Viewport-scoped highlight query producing semantic token spans; plain-text fallback with notice
- [x] 3.4 Tests: each language highlights a sample, incremental edits match full reparse, pathological nesting falls back, hostile tree does not panic

## 4. Engine: load, save, state

- [x] 4.1 Streaming load job on the file's lane with progress/cancel; first-screen readiness; 1 GiB limit and binary/archive read-only rules
- [x] 4.2 Save job: temp sibling + rename where supported, in-place otherwise, permissions preserved, temp cleanup on failure, owner-only temp
- [x] 4.3 Baseline (mtime/size) check, conflict decision flow (Reload / Overwrite / Save as), watcher reload for clean buffers and changed-on-disk bar for dirty ones (Reload / Keep mine; Compare left out, see design D10)
- [x] 4.4 `Session` editor state, dirty tracking, autosave triggers (pane focus, window blur event, file change, pane close, exit via close guard) and the autosave-off warning
- [x] 4.5 `editor_cmds.rs` with all `edit.*` commands, registered in `commands.rs`, with malformed-params fuzz tests and wrong-pane tests
- [x] 4.6 Preferences: `editor_open` per kind, `editor_autosave`, `editor_undo_steps`, `syntax` settings; `prefs.editor_open`, `prefs.editor_autosave`, `prefs.editor_undo_steps` commands; persisted in `Prefs` (no secrets)
- [x] 4.7 In-memory `Vfs` tests: load/save/conflict/autosave on local, SMB and SFTP-shaped fakes; failed-save keeps buffer dirty
- [x] 4.8 `editor.open` / F4 (`editor.toggle`) pane selection logic (follow pane, any Preview, else split) and `editor.open_external`

## 5. Layout state

- [x] 5.1 Preview pane state gains `edit: Option<EditViewState>` (caret, selection, scroll, wrap); legacy state files load as preview; workspaces persist file/caret/scroll but not content
- [x] 5.2 Follow-source rules while dirty; layout tests for split/close invariants with edited panes

## 6. UI widget (`fm-ui`)

- [x] 6.1 Read-only virtualized text view over `Buffer` with line numbers, per-line galley cache, long-line windows, soft wrap
- [x] 6.2 Caret, selection, pointer and keyboard input, clipboard, IME, scroll-into-view
- [x] 6.3 Editing wiring through `editor.input`; bracket-match highlight; indent guides off (not in scope)
- [x] 6.4 Find/replace bar, jump-to-line prompt, conflict dialog, dirty-buffer warning with autosave hint, "changed on disk" bar, large-file notices
- [x] 6.5 Semantic syntax tokens in `Tokens::dark()`/`light()`; no `Color32::` literals outside `theme.rs`
- [x] 6.6 Toolbar (pencil toggle, mode and dirty markers, external-editor button), context menu entries
- [x] 6.7 `egui_kittest` snapshots in dark and light for plain, coloured, find bar, warnings; soak/perf test with 100 MB buffer scroll

## 7. App wiring and preferences UI

- [x] 7.1 F4, Ctrl+G, Ctrl+F (replace is a toggle in the bar: Ctrl+H stays hidden files), Ctrl+S, undo/redo shortcuts and menu entries in `fm-app`; window-blur event forwarded to autosave trigger
- [x] 7.2 Preferences → Editor page (open per kind, autosave, undo steps, wrap, line numbers) and Enter/double-click routing in Files panes and the tree

## 8. Docs and wrap-up

- [x] 8.1 Update `docs/architecture.md` (new crate, tree-sitter exception, editor data flow) and `docs/roadmap.md`
- [x] 8.2 `CHANGELOG.md` entry under Unreleased; `log/devlog.md` entry with test counts and measurements
- [x] 8.3 Run full gate: `cargo test` for touched crates, clippy `-D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`, `cargo deny check`
