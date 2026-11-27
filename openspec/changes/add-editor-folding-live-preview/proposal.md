## Why

With the core editor in place (`add-text-editor`), two things keep it from feeling like a real code editor: navigating long files needs **code folding**, and reading nested code needs **rainbow brackets**. Markdown, HTML and SVG authors also want to see the result while typing, without adding a pane type. A second Preview pane can already render a file; it only needs to follow an editor's live, unsaved buffer instead of the disk.

## What Changes

- **Code folding** in Edit mode: fold gutter with chevrons, fold/unfold at the caret, fold all / unfold all, fold level, folded-region placeholder (`… N lines`), based on tree-sitter node ranges for supported languages and on indentation for the rest. Fold state survives edits and is remembered per file for the session.
- **Rainbow brackets**: matching bracket pairs coloured by nesting depth from a cyclic palette of semantic theme tokens (dark and light), skipping strings and comments, with a preference to turn it on or off (default on) and a depth-cycle length.
- **Live preview**: a Preview pane can take an Edit-mode Preview pane as its source and render the live buffer (unsaved text included), debounced, for Markdown, HTML, SVG, Mermaid, JSON/YAML/TOML/XML pretty views and tables. `editor.open_live_preview` opens or focuses such a pane beside the editor and wires the source. Scroll sync is left for a later change.
- Folding, rainbow and live preview all degrade with the same large-file rules as syntax colouring.

## Capabilities

### New Capabilities
<!-- none: the editor capability gains requirements -->

### Modified Capabilities
- `text-editor`: adds code folding, rainbow brackets and live-preview source behaviour.
- `preview-pane`: a Preview pane may follow an Edit-mode Preview pane as its source (extends source rules and `pane.preview.source`).

## Impact

- `fm-editor` (L1): fold model (ranges from tree-sitter nodes, indentation fallback, fold state mapped through edits), bracket-depth pass.
- `fm-preview` (L1): render models accept an in-memory text source with a version counter, not only a `Vfs` entry.
- `fm-layout` (L2): preview source can reference an editor pane; fold state and rainbow toggles in edit view state.
- `fm-engine` (L3): new commands in `editor_fold_cmds.rs` and `editor.open_live_preview`; debounced buffer snapshots published to followers; preferences `editor_rainbow`.
- `fm-ui` (L4): fold gutter, placeholders, bracket colours, theme tokens in both themes, kittest snapshots.
- `fm-app` (L5): shortcuts and menu entries.
- No new dependencies; reuses the tree-sitter grammars from `add-text-editor`. Depends on that change being implemented first.
