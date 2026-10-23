## 1. Pane kind model

- [ ] 1.1 `fm-layout`: `PaneKind` (+ serde, `as_str`/`parse`), `PaneState.kind` default Files; tests
- [ ] 1.2 `engine`: `pane_kind_cmds.rs` — `pane.kind.set` (validation, placeholder kinds error), `Session::focused_kind()`; register in `commands.rs`; malformed-param fuzz
- [ ] 1.3 Kind-aware `enabled` for `file.*`, `nav.*`, `dnd.*`, F5/F6 destination, drop zones; tests (copy disabled on Terminal pane)
- [ ] 1.4 Files state round-trip test (location/history/selection/tree)

## 2. `fm-term` crate (L1)

- [ ] 2.1 Scaffold crate, register `fm-term` at L1 in `xtask/src/layers.rs`; deps `portable-pty`, `vt100`
- [ ] 2.2 `TermSession`: spawn (cwd, size, sanitised env, login shell), reader thread + bounded channel, writer channel, resize, status/exit code, kill on Drop
- [ ] 2.3 Parser pump with per-frame byte budget; `Screen` snapshot (cells, attrs, cursor, scrollback offset); scrollback cap
- [ ] 2.4 Tests (skip without pty): echo round-trip, resize, exit code, child killed on drop, flood stays within budget, garbage/OSC fuzz never panics
- [ ] 2.5 Pure `shell_quote_cd(path)` for POSIX/Windows with hostile-name tests

## 3. Engine integration

- [ ] 3.1 `Session.terminals`, lazy spawn on kind switch, removal on `pane.close`, shutdown kill; `poll()` pumps terminals and requests repaint
- [ ] 3.2 `term_cmds.rs`: `term.input` (hidden, capped), `term.copy`, `term.paste` (1 MiB cap), `term.cd_here`, `term.clear_scrollback`, `term.restart`; enabled predicates; bad-params tests
- [ ] 3.3 Start directory: pane location if local else home; test with `MemFs` location falls back

## 4. UI

- [ ] 4.1 Theme tokens: terminal fg/bg/cursor/selection + 16 ANSI colours (dark + light, exhaustive); pure 256/true-colour mapping
- [ ] 4.2 `fm-ui::terminal`: grid painting (runs, cursor, inverse/bold/underline), resize → rows/cols, wheel scrollback, selection, exit banner with Restart
- [ ] 4.3 Pure key→bytes translation (xterm, application cursor mode) with table tests; focus-lock filter for Tab/arrows/Esc
- [ ] 4.4 Pane header kind selector (Files / Terminal / Sftp·Network disabled) dispatching `pane.kind.set`; View ▸ Pane Type menu
- [ ] 4.5 Snapshots: terminal pane dark + light, selector open

## 5. App wiring

- [ ] 5.1 `fm-app`: `terminal_focused` gate in `consume_shortcuts` (only Ctrl+Shift chords), `Ctrl+Shift+Tab` swap, `Ctrl+Shift+C/V`; menu + `all_ids` entries; shortcut tests
- [ ] 5.2 Quit/pane-close kill path; repaint cadence while a terminal is visible

## 6. Docs and gate

- [ ] 6.1 `docs/architecture.md` (pane kinds, terminal data flow, shortcut routing), `docs/development.md` smoke list, roadmap (change 4 builds on pane kinds)
- [ ] 6.2 Whole `cargo xtask ci` green; manual smoke (shell, `vim`/`htop`, resize, copy/paste, Ctrl+C) by user; `log/devlog.md` entry
