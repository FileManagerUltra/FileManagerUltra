## Why

Preview panes have their own zoom (− 100% + in their toolbar), terminals only follow the global terminal font size, so making one terminal bigger (a log on a shared screen, a small split) changes every terminal (requested 2026-11-14). A first version put a floating − 100% + handle over the terminal; the user did not like controls floating over the content and asked for them in the pane header, for every pane type, hidden behind one icon so they are not always in the way (2026-11-15).

## What Changes

- Each terminal pane gets its own text zoom (50–400%, ×1.2 steps), on top of the terminal font size and independent of other terminals and the interface zoom; it is stored with the pane, so it travels with the workspace.
- Commands `term.zoom.in`, `term.zoom.out`, `term.zoom.reset` (`{pane?}`); while a terminal has focus, the interface zoom chords Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 zoom that terminal instead (as Preview does with its zoom keys). Ctrl+= / Ctrl+- stay with the shell.
- Every pane header gets a magnifier left of the pane type selector; it opens − 100% + stacked vertically. Commands `pane.scale.in/out/reset {pane?}` route to the terminal zoom, the preview zoom, or a new per-pane zoom for Files, Network, SFTP, Trash and Media panes (text, row height and spacing of that pane; saved with the pane). No new shortcuts.
- The Preview toolbar no longer has its own − 100% + (Fit / 100% for pictures stay).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-pane`: new requirement "Per-terminal zoom".
- `panes`: new requirement "Pane zoom".
- `preview-pane`: "Preview has pane-local Find" and "Pane-local preview magnification" (zoom moves from the toolbar to the pane header).

## Impact

`fm-layout` `PaneState::term_scale` and `view_scale` (serde default 1.0, read bounded), `crates/engine/src/term_zoom_cmds.rs`, `crates/engine/src/pane_scale_cmds.rs`, `crates/ui/src/pane.rs` (header magnifier + popup, scaled body), `crates/ui/src/theme.rs` (`Tokens::scaled`, `scale_style`), `crates/ui/src/media_icons.rs` (magnifier icon), `crates/ui/src/terminal.rs` (scaled grid), `crates/ui/src/preview.rs` (toolbar buttons removed), `apps/fm-app/src/menu.rs` (chord mapping), shortcuts reference. No new dependencies.
