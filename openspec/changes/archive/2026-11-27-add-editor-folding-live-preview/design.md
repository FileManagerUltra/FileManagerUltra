## Context

`add-text-editor` provides the rope `Buffer`, an incremental tree-sitter parse per buffer on a worker pool, a virtualized widget and Edit mode inside Preview. Preview panes follow a source Files pane today; models in `fm-preview` read from a `Vfs` entry. This change adds folding, rainbow brackets and live preview on top, with the same constraints: no new pane type, nothing blocks the UI thread, large files degrade gracefully.

## Goals / Non-Goals

**Goals:**
- Accurate folding for supported languages, still useful for the rest.
- Rainbow brackets that ignore strings and comments and stay cheap on big files.
- Live preview using an ordinary Preview pane, updating from the unsaved buffer.

**Non-Goals:**
- Fold persistence across app restarts, custom fold markers/regions, folding inside the preview.
- Scroll-synced split for formats without a line mapping.
- Collaborative or multi-buffer live preview (one source editor per follower).

## Decisions

**D1. Fold ranges come from the syntax tree, with an indentation fallback.** For each grammar a small query (`folds.scm`-style, bundled per language) marks foldable nodes (blocks, functions, classes, arrays/objects, Markdown sections, XML/HTML elements, comments). A range is foldable if it spans at least 2 lines. Languages without a query, plain text and files above the syntax threshold use indentation (a line folds up to the next line with indentation not greater than its own, ignoring blank lines). Ranges are recomputed on the worker after each incremental parse and for the visible window only above a size threshold; between parses the existing ranges are shifted through edits so folds do not jump while typing. *Alternative:* brackets only — rejected, misses Python/YAML/Markdown.

**D2. Fold state is a set of start lines mapped through edits.** An edit inside a folded range unfolds it; edits above shift it. Rendering maps buffer lines to display lines via a fold index (sorted disjoint folded ranges, binary search), so cost per frame depends on visible lines, not file size. Caret movement into a folded range unfolds it; copy of a selection spanning a fold includes the hidden text. State is kept in `EditViewState` for the session and in the workspace snapshot only as start lines, dropped if invalid.

**D3. Rainbow brackets use the highlight pass, not a second parser.** The highlight query already marks bracket punctuation nodes; the bracket pass walks only the visible range plus margin, but needs depth at the range start, so the worker keeps checkpoint depths per N lines (a sparse array updated incrementally from the tree). Depth `d` maps to palette token `d % cycle`; unmatched brackets use a dedicated error token. Without a tree (plain text or large files) rainbow is off rather than approximate, with the same notice style as colouring.

**D4. Live preview is a source reference, not a copy.** A follower Preview stores `PreviewSource::Editor(pane_id)`. The editor publishes a versioned snapshot (`Arc<str>`-like rope clone, cheap) after a debounce (default 250 ms, longer for big buffers) into the `Session` inbox; the follower re-renders only when the version changes. Rendering stays in the existing preview pipeline on its own worker, so typing never waits for it. `fm-preview` models gain an in-memory input (`text`, `version`, `origin_uri`) so relative links and images still resolve against the file's folder. The input is `fm_preview::snapshot::SnapshotVfs`: a read-only single-file `Vfs` holding the text under the file's own uri, so every renderer (and table paging) works unchanged. Superseded renders are not cancelled: their results carry an old slot token and are dropped. Every type renders as the Preview pane would show it (code as the text view), so there is no "unsupported" state.

**D5. Scroll sync is deferred.** (Not built in this change; kept for a later one.) Formats with a source-line map (Markdown blocks, and sources whose renderer reports line ranges) scroll the follower to the caret's block when the caret moves, unless the user scrolled the follower manually in the last few seconds. Formats without a map do not sync.

**D6. Wiring and lifecycle.** `editor.open_live_preview` finds a Preview pane that already follows this editor, else any other Preview pane, else splits one beside it, sets its source and returns focus to the editor. Closing the editor reverts the follower to following focus; toggling the editor back to preview, or switching file, keeps the follower pointing at the pane (it re-renders the new buffer). A Preview cannot follow itself, and chains (follower of follower) are refused to avoid cycles. `pane.preview.source` accepts an Edit-mode Preview pane in addition to Files panes.

## Risks / Trade-offs

- [Fold/bracket work on huge files] → viewport-scoped queries, sparse depth checkpoints, thresholds shared with syntax, off above them.
- [Fold ranges stale between parses] → shift through edits, recompute on parse result; worst case a fold is dropped, never text.
- [Live preview cost on large Markdown] → debounce scaling with size, renderer already off the UI thread, cancel superseded renders.
- [Per-language fold queries are extra maintenance] → small bundled queries, fallback to indentation when absent, tests per language.
- [Source reference cycles or dangling panes] → validate on every layout change; revert to following focus.

## Migration Plan

Additive. Saved panes without a source reference or fold state load as before. Rainbow default on; preference to disable. No data format changes beyond optional fields.

## Open Questions

- Default rainbow cycle length (assume 6 colours) and palette accessibility check for dark and light themes.
- Whether fold-all should respect a maximum level shortcut set (Ctrl+K Ctrl+1..) or only fold all/unfold all in this change (assume the latter).

## Implementation notes (2026-11-26)

- Fold ranges come from node-kind name rules over the syntax tree (`fold::tree_ranges`, one list for every grammar) instead of per-language query files: smaller, and every curated grammar gets folding without extra assets. Indentation ranges are worked out lazily per line, so large files only pay for the lines shown.
- Rainbow depth is the count of enclosing bracket pairs in the tree for the visible lines; no checkpoints are needed because highlighting already queries per visible window. A bracket whose partner the parser invented (`MISSING`) is unmatched and uses the error colour.
- The folded placeholder is `… N lines` (U+2026): the bundled fonts have no `⋯`.
- No Ctrl+K V: the shortcut table has no two-key sequences; Live Preview is on the toolbar, the File › Editor menu and the source picker.
- `editor_holds` (dirty editor vs. source selection) arms on the first subject it sees, like `editor_follow`: a Files listing that arrives after the editor opened is not a move and must not autosave (found by the live-preview snapshot under CPU load).
