## 1. Fold model (`fm-editor`)

- [x] 1.1 Fold ranges from the syntax tree for the supported grammars (blocks, functions, classes, objects/arrays, Markdown sections, markup elements, block comments) — node-kind rules, not query files (design: implementation notes)
- [x] 1.2 Indentation fallback fold ranges (blank-line aware), min 2 lines
- [x] 1.3 `FoldIndex`: sorted disjoint folded ranges, buffer-line to display-line mapping by binary search, shift through edits, unfold on edit inside
- [x] 1.4 Recompute ranges on the parse worker after each parse; indentation ranges lazily per visible line (large files)
- [x] 1.5 Tests: each language yields expected ranges, edit shifting, unfold rules, 1M-line index lookups, malformed lines return `Err`

## 2. Rainbow brackets (`fm-editor`)

- [x] 2.1 Bracket depth from enclosing pairs in the tree for the visible lines, skipping strings/comments (no checkpoints needed: per-window like highlighting)
- [x] 2.2 Unmatched-bracket detection and depth to palette index mapping
- [x] 2.3 Tests: nesting, strings/comments, edits that change depth downstream, off without tree

## 3. Engine

- [x] 3.1 `editor_fold_cmds.rs`: `editor.fold`, `editor.unfold`, `editor.toggle_fold`, `editor.fold_all`, `editor.unfold_all` with malformed-params fuzz tests and wrong-pane tests
- [x] 3.2 Caret/find/goto-line unfold hooks and copy-through-fold
- [x] 3.3 `prefs.editor_rainbow {enabled}` and preference persistence; fold state in `EditViewState` (session) and workspace snapshot as start lines, validated on load
- [x] 3.4 Versioned, debounced buffer snapshots rendered for followers (`editor_live`); size-scaled debounce; superseded renders dropped by token
- [x] 3.5 `PreviewSource::Editor(pane)` handling: `pane.preview.source` validation (no self, no follower chains, valid kinds), revert on editor close, layout-change validation
- [x] 3.6 `editor.open_live_preview`: reuse follower, else any other Preview, else split; focus back to editor

## 4. Preview rendering (`fm-preview`)

- [x] 4.1 In-memory render input: `SnapshotVfs` (text under the file's uri) for every renderer incl. tables; relative links/images resolve against the origin folder
- [x] 4.2 ~~Source-line maps / scroll sync; "No live preview" state~~ deferred: scroll sync left for a later change, every type renders as the Preview pane shows it (design D4/D5)
- [x] 4.3 Tests: version change triggers re-render, unchanged version does not, oversized buffers are refused (8 MiB)

## 5. UI (`fm-ui`) and app (`fm-app`)

- [x] 5.1 Fold gutter with chevrons, folded placeholder `… N lines`, folded lines skipped in layout and scrolling
- [x] 5.2 Rainbow bracket palette tokens in `Tokens::dark()`/`light()` (cycle of 6 plus error token), contrast checked in both themes
- [x] 5.3 Toolbar (Live), context menu and File › Editor entries for fold and live preview; Ctrl+Shift+[ / ] (no Ctrl+K V: no key sequences); Preferences → Editor rainbow toggle
- [x] 5.4 Header source picker lists Edit-mode Preview panes; follower shows source name
- [x] 5.5 `egui_kittest` snapshots in dark and light: folded region with rainbow nesting, live preview split; the 400 000-line scroll soak covers the fold-aware layout

## 5b. Look (user feedback 2026-11-26)

- [x] 5b.1 Painted icon set for the editor toolbar, find bar and go-to bar (`editor_icons`), overflow menu with text items
- [x] 5b.2 Line spacing preference (`prefs.editor_line_spacing`, 100–200 %, default 145 like the code preview); caret, gutter and fold marker centred in the row
- [x] 5b.3 Code colour schemes (`prefs.editor_scheme`: App, Warm, High contrast; dark + light), `Tokens.code`, used by editor and code preview; contrast tests (4.5:1, 7:1 for high contrast incl. rainbow); Preferences cards with samples; italic comments

- [x] 5b.4 Preferences: "Opening files" for the editor and for media as an aligned table with segmented choices (row-qualified accessible names, one Tab stop, arrow keys); editor topic starts with colours and spacing
- [x] 5b.5 One toolbar for every preview and Edit mode (`pane_bar`): Wrap, Find, Edit/Preview fixed at the right, view tools as icons left of them (fold into View options), Save/Fill & Sign pinned; logs search via Find; tables use the Source icon; Diff toggles
- [x] 5b.6 Scrolling: same wheel distance in preview and editor, Scroll speed preference, smooth editor scrolling, slim widening scroll handles

## 6. Docs and wrap-up

- [x] 6.1 Update `docs/architecture.md` (fold index, follower snapshots) and `docs/roadmap.md`
- [x] 6.2 `CHANGELOG.md` entry under Unreleased; `log/devlog.md` entry with test counts and measurements
- [x] 6.3 Full gate: `cargo test` for touched crates, clippy `-D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`
