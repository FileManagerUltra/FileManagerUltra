## Context

Panes are `fm-layout::PaneState` (location, history, listing, tree) rendered by `fm-ui::pane`. Everything below L4 is egui-free, the UI thread never blocks, and every behaviour is a command. Change 4 needs panes that are not file views (SFTP connect/browse, network shares); the user also wants a shell beside the files. Terminal is built first; Sftp/Network are reserved.

## Goals / Non-Goals

**Goals:**
- A `PaneKind` model that later kinds plug into without touching Files code.
- A robust terminal pane: pty, colours, scrollback, resize, copy/paste, bounded against hostile output.
- Zero regressions for Files panes (all existing tests and snapshots unchanged).

**Non-Goals:**
- Sftp/Network bodies (change 4); terminal tabs inside a pane; ligatures, sixel/images, hyperlinks, mouse reporting to apps (v1: wheel scrollback + selection only); persisting terminal sessions across restarts; syncing the shell cwd back to the file pane.

## Decisions

**D1 — Kind lives in layout, per-kind state alongside.** `PaneState.kind: PaneKind` with the Files fields untouched (kept while hidden). Terminal state is *not* in layout (it holds threads/handles); layout only knows the kind. Alternative: enum-of-state (`PaneBody::Files(..)`) — rejected: forces every Files access through a match and loses state on switch.

**D2 — New L1 crate `fm-term`.** `TermSession::spawn(cwd, size) -> Result<_, FmError>`, `write(&[u8])`, `resize(rows, cols)`, `snapshot() -> Screen` (cells, cursor, scrollback offset), `status() -> Running | Exited(code)`, `Drop` kills the child. Depends on `portable-pty` + `vt100` only; no egui, no vfs I/O. L1 because it is process I/O below the engine. Alternative: `alacritty_terminal` — rejected as heavy with an unstable API; `vt100` is small and fuzz-friendly.

**D3 — Reader thread + bounded channel.** A thread reads the pty into `Vec<u8>` chunks (≤ 64 KiB) on a bounded channel; `poll()` feeds the parser up to a per-frame byte budget (256 KiB) and requests a repaint when more remains. Writes go through a second small channel so keystrokes never block on a full pty. Alternative: async pty — rejected (tokio is confined to `fm-sftp`).

**D4 — Engine owns sessions.** `Session.terminals: HashMap<PaneId, TermSession>`, created lazily by `pane.kind.set terminal` (spawn failure → notice + kind unchanged), removed on `pane.close`, killed on shutdown. The UI gets a read-only `Session::terminal(pane) -> Option<&TermSession>` plus commands to write; input from the UI is dispatched via `term.input {bytes}` (internal, hidden from menus, size-capped), keeping "UI never calls a backend".

**D5 — Rendering in `fm-ui::terminal`.** Monospace cell metrics from the theme font; paint backgrounds per run, then text per run (not per cell) to stay cheap; default fg/bg/cursor/selection and the 16 ANSI colours are new semantic tokens in `Tokens::dark()/light()` (exhaustive; no `Color32::` outside `theme.rs`). 256/true colours are computed values (a pure function in `theme.rs`). Size → rows/cols each frame; `resize` only on change.

**D6 — Shortcut gating.** `consume_shortcuts` gets a `terminal_focused` flag next to `typing`: when set, only shortcuts with Ctrl+Shift pass (and `Ctrl+Shift+Tab` maps to `pane.swap`). Key → byte translation is a pure function in `fm-ui::terminal` (xterm sequences incl. application-cursor mode read from the parser state) and is unit-tested. egui's focus-lock filter claims Tab/arrows/Esc for the terminal widget (as the tree does).

**D7 — Hostile output.** `vt100` is configured with a scrollback cap (10 000); unknown sequences are ignored by the parser; we additionally cap the title string, never act on OSC 52/8, and feed in bounded slices. A fuzz-style test pushes random bytes and giant OSC payloads.

**D8 — cd_here quoting.** POSIX: single-quote with `'\''` escaping; Windows `cmd`: `cd /d "…"` with `"` rejected (invalid in paths anyway). Pure function with tests for hostile names.

**D9 — Kind-aware enabled predicates.** A helper `Session::focused_kind()`; file/nav/dnd commands add `kind == Files` to `enabled`. Existing tests use Files panes, so behaviour is unchanged.

## Risks / Trade-offs

- [UI stalls from output floods] → per-frame byte budget + repaint request; flood test.
- [Orphaned child processes] → kill on `Drop`, on pane close and app exit; test spawns `sleep` and asserts it is gone.
- [App shortcuts steal terminal keys / terminal steals app keys] → explicit Ctrl+Shift escape hatch, documented; routing is a pure tested function.
- [Windows ConPTY quirks, macOS login shell] → compiled in CI on all three; terminal tests skip when no pty can be created (like GPU snapshots).
- [`vt100` feature gaps (mouse reporting, alt-screen quirks)] → documented non-goal; apps like `vim`/`htop` work, mouse in them does not.
- [Selection/copy on a moving grid] → selection stores screen coordinates and clears on scroll/resize in v1.

## Migration Plan

Additive. `PaneState` gains `kind` (default `Files`); existing prefs/state files have no pane kinds so nothing migrates. Rollback = remove the crate and kind selector.

## Open Questions

- Should a terminal pane's kind and cwd persist in change 5's session persistence? Deferred to change 5 (content never persisted regardless).
