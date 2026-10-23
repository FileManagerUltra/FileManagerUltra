## 1. Layout model

- [x] 1.1 `Edge` enum; `PaneNode::detach_leaf`, `insert_beside`, `rotate_parent`, `equalize`, `nudge`; `Layout::move_pane/equalize/rotate/resize`
- [x] 1.2 `Layout.zoomed` + `fix_zoom()` on every mutation; zoom toggle
- [x] 1.3 Unit tests (move to each edge, centre swap, invalid, equalize thirds, rotate, resize clamp) and a randomised invariant test

## 2. Engine commands

- [x] 2.1 `arrange_cmds.rs`: `pane.move`, `pane.equalize`, `pane.zoom`, `pane.rotate`, `pane.resize.*`; register in `commands.rs`
- [x] 2.2 Terminals and per-pane state survive moves; zoom ends on focus change; malformed-param tests

## 3. UI

- [x] 3.1 `fm-ui::arrange`: `PaneDrag` payload, `slot_for`, overlay painting (theme tokens), drop dispatch
- [x] 3.2 Border drag source in `pane.rs`; target overlay on every pane kind
- [x] 3.3 Divider widget in `fm-app`: hover colour/cursor, double-click reset, min size
- [x] 3.4 Renderer honours zoom
- [ ] 3.5 Tests: `slot_for` table, headless render of overlay (dark + light)

## 4. App wiring

- [x] 4.1 Panes menu entries; `Ctrl+Shift` chords work in terminal panes; shortcut tests
- [ ] 4.2 Brand-check / layers green

## 5. Docs and gate

- [ ] 5.1 `docs/architecture.md` (layout ops, zoom, drag payload), `docs/roadmap.md`, devlog
- [ ] 5.2 `cargo xtask ci` green
