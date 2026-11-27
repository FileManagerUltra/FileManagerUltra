## Context

The Preview pane (`fm-preview` models, `fm-ui` rendering, `preview_*_cmds.rs` commands) already shows text, code, Markdown, logs and tables, follows a source Files pane, and has pane-local Find. `PaneKind` has 7 kinds and the project wants to keep that number. Files are read through `Vfs`, with all I/O on engine job lanes; the UI thread never blocks. `syntect` is already in the lockfile for read-only code preview.

The editor must feel native for 1 GB files and for remote files, colour code accurately, and never lose user data. Speed and cross-platform support outrank the pure-Rust rule (tree-sitter exception approved 2026-11-26).

## Goals / Non-Goals

**Goals:**
- Edit mode inside Preview, no new `PaneKind`.
- Smooth editing and scrolling on very large files (virtualized, rope-backed, viewport-only highlighting).
- Same behaviour on local, SMB and SFTP.
- Autosave on focus loss with a preference; safe conflict handling.
- Per-kind internal/external editor preference and an always-available external editor command.

**Non-Goals:**
- Folding, rainbow brackets, live preview (next change `add-editor-folding-live-preview`).
- Multi-cursor, LSP, plugins, macros, split editing of one buffer, binary/hex editing.
- Replacing the existing read-only preview renderers.

## Decisions

**D1. Edit mode is a flag on the Preview pane state.** `PaneState` for a Preview pane gains `edit: Option<EditViewState>` (cursor, selection, scroll, wrap). Absent means preview. Old state files deserialize unchanged. *Alternative:* a new `PaneKind::Editor` — rejected (pane-type count). *Alternative:* rename Preview — rejected; preview stays the main purpose.

**D2. New L1 crate `fm-editor` with no egui types.** Contains `Buffer` (rope via `ropey`), `Edit`/`UndoTree`, `LineIndex`, `Encoding`/EOL detection, `Highlighter` and `Find`. The UI crate renders it; the engine owns instances in `Session`. This keeps it testable and respects layering. *Alternative:* egui `TextEdit` — rejected (O(n) layout, no virtualization).

**D3. Rope buffer, line-addressed API.** All positions are `(line, column-in-chars)` externally and byte/char offsets internally; every public method is bounds-checked and returns `Result`. Undo is a coalescing edit log (typing runs merge) bounded by a user-set step count (`editor_undo_steps`, default 1000, 10–100000) and a fixed 64 MiB text cap; beyond either the oldest groups drop and the UI says so.

**D4. Custom virtualized widget in `fm-ui`.** Only visible lines are laid out; galleys are cached per line keyed by `(line version, wrap width, theme)`. Very long lines are laid out as clipped windows around the caret/viewport (horizontal virtualization) above a length threshold. Scrolling uses a line-height prefix model; with wrap on, heights are estimated and refined as lines come into view. IME and clipboard go through egui events.

**D5. Highlighting: tree-sitter on a worker, viewport-scoped.** One `tree_sitter::Parser` per open buffer on a dedicated bounded pool. Edits are fed as `InputEdit` for incremental reparse; highlight queries run only over the visible byte range plus margin. Results are posted to the `Session` inbox and applied in `poll()`. Grammars sit behind cargo features (Rust, TOML, JSON, YAML, Markdown, shell, Python, JS/TS, C/C++, Go, Java, HTML/CSS, XML, SQL, PHP); language is chosen by extension then shebang, never executed content. Above `syntax_max_bytes` (default 8 MiB, from the spike) or when a parse exceeds its time budget, the buffer falls back to plain text and the UI says so. Parsing is cancellable via the parser progress callback. Colours come from new semantic tokens in `Tokens::dark()`/`light()`. *Alternative:* syntect — rejected for speed on large files; it stays for read-only preview until migrated.

**D6. Load and save are jobs.** Load streams chunks into the rope on the file's lane with progress and cancel; the editor shows the first screen as soon as available, and editing is enabled when load completes (read-only while loading). Save writes to a temp sibling and renames where the backend supports it, otherwise writes in place; it records the resulting `(mtime, size)` as the new baseline. Before saving, the engine stats the file; a differing baseline raises a conflict decision (reload, overwrite, save as) through the existing decision-channel pattern — autosave never overwrites silently. Edit limit is 1 GiB (above: view-only, external editor offered). Binary content (NUL bytes in the first 64 KiB, or invalid for the detected encoding) opens read-only with an offer for the external editor. Encoding (UTF-8 with/without BOM, UTF-16 LE/BE, Latin-1 fallback) and EOL style are detected and preserved on save.

**D7. Commands.** `editor.toggle` (F4), `editor.open {uri, line?, pane?}`, `editor.save` (Ctrl+S), `editor.save_as {uri, overwrite?}`, `editor.revert`, `editor.goto_prompt` (Ctrl+G) / `editor.goto_line`, `editor.find` (Ctrl+F; replace is a toggle in the bar) / `editor.find_next` / `editor.find_previous` / `editor.replace {all?}` / `editor.find_close`, `editor.undo`/`redo`, `editor.toggle_wrap`, `editor.toggle_line_numbers`, `editor.open_external`, `editor.window_blur`, `editor.resolve {choice}`, and `prefs.editor_open {kind, target}`, `prefs.editor_autosave {enabled}`, `prefs.editor_undo_steps {steps}`, `prefs.editor_view`, `prefs.editor_syntax`. The prefix is `editor.`, not `edit.`, because `edit.undo` and `edit.select_all` already exist for files. Pure text-manipulation inside the widget (typing, cursor motion) is dispatched as `editor.input {op}` with a bounded enum so the control channel can drive it; pastes over the 1 MiB parameter cap go through `Session::editor_input_text`. All are no-ops unless the focused pane is a Preview in Edit mode and all take malformed-params fuzz tests. Shortcuts follow the "same chord everywhere" rule: while the editor has focus it types like a text field (Ctrl+Z/Y/A/C/X/V, Tab, Backspace, Delete are the editor's); editor-only chords use free keys (Ctrl+/ comment, Alt+Up/Down move lines, Alt+Shift+Down duplicate, Ctrl+Shift+K delete line); Ctrl+H stays "hidden files".

**D8. Autosave triggers** are evaluated in `Session`: pane focus moves away from an edited Preview pane, the OS window loses focus (event from `fm-app`), the pane's file changes (source follow, navigation, `editor.open` of another file), pane close, and app exit (via the existing close guard). With autosave off, those same triggers raise the dirty-buffer warning (Save / Discard / Cancel) whose text names the autosave preference, except the window losing focus (switching applications is not leaving the editor). Leaving Edit mode (F4 / Preview) counts as leaving: it saves first, or warns. Cancel remembers the file the user tried to move to, so following the source does not raise the warning again for the same file. With autosave on, closing a pane or quitting saves edited files first and then continues with the existing close guard. A failed autosave keeps the buffer dirty, reports an actionable error and never discards edits.

**D9. Open preference** mirrors `media-open-preference`: `Prefs::editor_open` maps file kinds (text, code, markup, config, log, other-known) to `internal | external`, default `internal` for recognised text kinds. Recognition reuses the preview type sniffing plus the preview filename filters. Remote files, archive members and files over the limit follow the same rules with the existing copy-first behaviour for external opens.

**D10. Watcher interplay.** If the file changes on disk while open and the buffer is clean, it reloads silently (cursor and scroll preserved); if dirty, the pane shows a non-modal "changed on disk" bar with Reload / Keep mine (the next save then replaces the file); Compare is left out of this change. A deleted file shows the same bar. Symlinks are followed (at most 8 hops) and the file they point to is read and saved; the link itself stays.

## Risks / Trade-offs

- [Custom text widget is the largest risk: IME, selection, wrapping, accessibility] → vertical slice first (read-only virtualized view, then caret, then editing), kittest snapshots in both themes, soak tests with 100 MB/1 GB files.
- [tree-sitter C code increases build requirements and binary size] → curated grammars behind features, spike measuring binary size/CI time on 3 OSes before enabling the full set; runtime fallback to plain text.
- [Rope + undo memory on 1 GB files] → undo cap, no per-line allocation beyond viewport caches, load streams in chunks.
- [Parser on hostile input (deep nesting, pathological files)] → size threshold, time budget, cancel, run off the UI thread; parser crash is not catchable, so grammars are limited to well-maintained crates and fuzzed with the hostile tree.
- [Autosave overwriting a remote edit] → baseline check before every save; conflict prompt, never silent overwrite.
- [Remote latency on load/save] → chunked streaming with progress, cancel on close, jobs on the remote lane only.
- [Window-blur autosave on platforms with flaky focus events] → debounce and also save on pane focus changes; covered by session tests that drive the triggers directly.

## Migration Plan

Additive. Pane state files without `edit` load as preview. New preferences default to internal editor for text kinds, autosave on. Rollback: removing the feature leaves unknown fields ignored by serde. Tree-sitter/ropey are added to `deny.toml` checks; notices for tree-sitter and grammars go into `fm_preview::notices` and `THIRD-PARTY-NOTICES.md`.

## Spike results (2026-11-26, Linux x86_64, release, thin LTO, tree-sitter 0.27)

- **Build:** cold build of runtime + 17 grammars 15 s on this machine; only needs `cc`. macOS and Windows are to be confirmed by the CI matrix when the dependency lands (task 1.2).
- **Binary weight** (stripped scratch binary): runtime ≈ 0.3 MB total baseline; grammars add roughly cpp 3.4 MB, typescript 2.9, php 2.2, bash 1.4, rust 1.2, markdown 0.9, c 0.6, python 0.5, js 0.45, java 0.45, go 0.24, yaml 0.22, css/xml 0.13 each, toml/html/json < 0.05. All 17 together add ≈ 13.4 MB. `tree-sitter-sequel` (SQL) alone adds ≈ 13.4 MB, so it is **excluded**; `tree-sitter-sql` 0.0.2 is 0.08 MB and is the SQL candidate, its highlight quality still to be verified.
- **Speed (Rust source, one worker thread):** viewport highlight query 0.5–0.7 ms at any file size. Incremental reparse after a one-line edit ≈ 26 ms at 1 MB, 220 ms at 8 MB, 830 ms at 32 MB, 3.4 s at 128 MB (it scales with size, since the root node has very many children). Full parse ≈ 0.2 s at 1 MB, 1.6 s at 8 MB, 7 s at 32 MB, 25 s at 128 MB. Highlight query compilation for all grammars ≈ 150 ms in total, so queries are compiled lazily per language.
- **Decisions from the numbers:** `syntax_max_bytes` = 8 MiB (above it: plain text); parse budget 5 s per full parse and 500 ms per incremental parse, both cancellable, after which the buffer falls back to plain text; highlight runs per viewport only; grammar set is cargo-feature gated and `sql` uses the small grammar. Cheaper heavy grammars (cpp, typescript, php, bash) stay in the default set because the user listed them; the feature gates let a lean build drop them.

## Open Questions

- Verify `tree-sitter-sql` colouring quality and, on the CI matrix, macOS/Windows build times and binary-size deltas.
- Whether `fm-preview`'s read-only code view should later share the tree-sitter highlighter and drop syntect (not in this change).
- Default F4 behaviour when the Files pane has no neighbouring Preview: reuse the last Preview pane or split (assume: reuse existing, else split right).
