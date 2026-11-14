## Why

Preview panes have their own zoom (− 100% + in their toolbar), terminals only follow the global terminal font size, so making one terminal bigger (a log on a shared screen, a small split) changes every terminal (requested 2026-11-14).

## What Changes

- Each terminal pane gets its own text zoom (50–400%, ×1.2 steps), on top of the terminal font size and independent of other terminals and the interface zoom; it is stored with the pane, so it travels with the workspace.
- A zoom handle, − 100% +, in the terminal's top-right corner: shown while the pointer is over the terminal and whenever the zoom is not 100%. The scrollback indicator moves below it while it is shown.
- Commands `term.zoom.in`, `term.zoom.out`, `term.zoom.reset` (`{pane?}`); while a terminal has focus, the interface zoom chords Ctrl+Shift+= / Ctrl+Shift+- / Ctrl+Shift+0 zoom that terminal instead (as Preview does with its zoom keys). Ctrl+= / Ctrl+- stay with the shell.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `terminal-pane`: new requirement "Per-terminal zoom".

## Impact

`fm-layout` `PaneState::term_scale` (serde default 1.0, read bounded), `crates/engine/src/term_zoom_cmds.rs`, `crates/ui/src/terminal.rs` (scaled grid, handle), `apps/fm-app/src/menu.rs` (chord mapping), shortcuts reference. No new dependencies.
