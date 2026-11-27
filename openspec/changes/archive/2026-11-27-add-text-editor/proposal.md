## Why

Users who browse files regularly need to fix a config, tweak a script or edit a Markdown note without leaving the app, and today they must hand the file to an external editor. The Preview pane already renders most text formats, so the natural home for editing is the same pane: preview stays the main purpose, and editing is a mode of it. No new pane type is added.

## What Changes

- Add an **Edit mode** to the Preview pane. `editor.toggle` (F4, File menu, Edit button in the Preview toolbar) switches the pane between the rendered preview and an editable text view of the same file. The pane kind stays `Preview`.
- Add a native, virtualized **code editor widget**: rope buffer, line numbers, jump to line, find/replace, undo/redo, bracket matching, code colouring via tree-sitter, and smooth handling of very large files (hard limit 1 GB; syntax features degrade above a lower threshold).
- Edit **local, SMB and SFTP** files identically through `Vfs` and the job lanes, with a changed-on-disk check and atomic save where the backend allows it.
- **Autosave** when the editor loses focus (pane focus change, window blur, file change, pane close, app exit). A preference turns it off; with it off, leaving a dirty buffer shows a warning that mentions the preference.
- **Open preference** per file kind (like media): activating a known text/code file opens it in the internal editor (default) or the external default app; *Open in external editor* is always available.
- `F4` on a Files-pane item opens it in Edit mode in an existing or new Preview pane.
- Approved dependencies: `ropey` (pure Rust) and `tree-sitter` with curated grammar crates behind cargo features (C, MIT; exception approved 2026-11-26 and recorded in `AGENTS.md`).
- Out of scope here, delivered by the follow-up change `add-editor-folding-live-preview`: code folding, rainbow brackets, and live preview in a second Preview pane.

## Capabilities

### New Capabilities
- `text-editor`: buffer, rendering, editing commands, find/replace, jump to line, syntax colouring, large-file behaviour, remote load/save, conflict detection, autosave and dirty-buffer handling.
- `editor-open-preference`: per-kind choice between the internal and the external editor, the `F4` entry point and the always-available external editor command.

### Modified Capabilities
- `preview-pane`: a Preview pane gains an Edit mode and its toolbar/menu/commands; preview commands defined for read-only mode stay unchanged.
- `security`: parse budget, size caps and hostile-input rules for the editor and the tree-sitter parsers; no credential is ever written to a buffer or temp file.

## Impact

- New crate `crates/editor` (`fm-editor`, L1: rope buffer, edit ops, undo tree, tokenizer/highlighter, no egui). Registered in `xtask/src/layers.rs`.
- `fm-layout` (L2): Preview pane state gains an edit-mode flag and view state (cursor, scroll); legacy state files still load.
- `fm-engine` (L3): new `editor_cmds.rs` (`editor.*`), load/save jobs on the file lanes, autosave and dirty tracking in `Session`, new preferences.
- `fm-ui` (L4): virtualized editor widget, theme tokens for syntax colours (dark and light), `egui_kittest` snapshots.
- `fm-app` (L5): F4 and shortcut/menu wiring, window-blur autosave trigger.
- Dependencies: `ropey`, `tree-sitter`, grammar crates; `cargo deny` licence check, binary-size and CI-time measurement on Linux, macOS and Windows.
- Docs: `docs/architecture.md`, `CHANGELOG.md`, `log/devlog.md`.
