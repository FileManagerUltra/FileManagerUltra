## Why

Panes can only show a file tree + listing. A file manager is more useful when a pane can also be a shell next to the files, and later a remote or network view, without leaving the multi-pane layout. The pane model must stop assuming "pane = files" before change 4 adds SFTP and network browsing.

## What Changes

- **Pane kinds**: every pane has a `PaneKind` (`Files`, `Terminal`, `Sftp`, `Network`). Switching is a command (`pane.kind.set {kind}`, plus View ▸ Pane Type and a small kind selector in the pane header). The Files state (location, history, selection, tree) is kept while another kind is shown, so switching back is lossless.
- **Terminal pane (implemented here)**: a pty-backed shell rendered in the pane. New L1 crate `fm-term` (pty + VT parser + cell grid, no egui, no filesystem calls); `engine` owns terminal sessions per pane and pumps output on a reader thread; `fm-ui` paints the grid with theme tokens (dark + light palettes) and forwards keys, paste, mouse wheel scrollback and resize.
- **Shell starts in the pane's directory** (local locations only; otherwise the home directory). `term.cd_here` sends a `cd` for the other pane's directory. Closing the pane or the app terminates the child.
- **Shortcut routing**: while a terminal pane is focused, keys go to the shell; only `Ctrl+Shift+…` chords and `Ctrl+Shift+Tab` (swap panes) stay app-level, so `F5`, `Tab`, `Backspace`, `Ctrl+L` etc. behave as in a terminal. `Ctrl+Shift+C/V` copy/paste.
- **Sftp and Network kinds (placeholders only)**: registered in the kind model and the selector but disabled with "arrives with network-backends". Their bodies are implemented in change 4 (`network-backends`), which now builds on this model.
- **Commands** (dispatched by id): `pane.kind.set`, `term.copy`, `term.paste`, `term.cd_here`, `term.clear_scrollback`, `term.restart`.

## Capabilities

### New Capabilities

- `pane-types`: the `PaneKind` model, switching, state preservation, placeholder kinds, selector and commands.
- `terminal-pane`: pty lifecycle, rendering, input routing, resize, scrollback, copy/paste, exit handling.

### Modified Capabilities

- `security`: terminal child environment, process cleanup and no-secrets-in-logs requirements (ADDED requirements only).

## Impact

- **Code:** new `crates/term` (L1); `crates/layout` (`PaneKind`, per-kind state); `crates/engine` (`pane_kind_cmds.rs`, `term_cmds.rs`, terminal session registry, `poll` pumping); `crates/ui` (`terminal.rs` widget, kind selector in `pane.rs`); `apps/fm-app` (shortcut gating, menu); `xtask/src/layers.rs` (register `fm-term` at L1).
- **Dependencies:** `portable-pty` (pty, cross-platform incl. Windows ConPTY) and `vt100` (parser/screen grid) in `fm-term` only. No `unsafe` in our code (golden rule 2).
- **Docs:** `docs/architecture.md` (pane kinds, terminal data flow, shortcut routing), `docs/development.md` (smoke list), roadmap (change 4 depends on this).
- **Risks:** blocking the UI thread on pty I/O (all reads on a thread, UI drains a bounded buffer); hostile output (escape sequences, huge lines, title/OSC strings) must be bounded; orphaned children on exit; global shortcuts stealing terminal keys.
