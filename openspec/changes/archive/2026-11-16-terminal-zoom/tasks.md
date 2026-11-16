## 1. Zoom

- [x] 1.1 `PaneState::term_scale` (serde default 1.0) with a bounded reader; bounds exported from fm-layout
- [x] 1.2 `term.zoom.in/out/reset {pane?}` in `term_zoom_cmds.rs`; tests (pane-local, bounds, prefs untouched, bad params, corrupt stored value)
- [x] 1.3 Terminal draws its grid with the scaled font; handle in the top-right corner (hover or not 100%), scrollback label below it; UI test clicking the handle
- [x] 1.4 Ctrl+Shift+= / - / 0 map to terminal zoom while a terminal has focus; test; shortcuts reference rows
- [x] 1.5 Reverted 1.4 (user, 2026-11-16: a shortcut must do the same everywhere): Ctrl+Shift+= / - / 0 are the interface zoom in every pane, Terminal and Preview included; `term.zoom.*` lose their shortcuts; Preview keeps Ctrl+= / - / 0; test `interface_zoom_chords_zoom_the_app_in_every_pane`; shortcuts reference, header tooltips, docs

## 2. Finish

- [x] 2.1 CHANGELOG, devlog; clippy, tests, layers, brand-check, fmt

## 3. Zoom in the pane header (all pane types)

- [x] 3.1 `PaneState::view_scale` (serde default 1.0, bounded reader); layout test (round trip, legacy, corrupt values)
- [x] 3.2 `pane.scale.in/out/reset {pane?}` in `pane_scale_cmds.rs`, routing Terminal/Preview to their own zoom; `pane_scale()` for the header; tests (every kind, bounds, routing, bad params)
- [x] 3.3 Header magnifier (painted `Icon::Zoom`) left of the pane type, vertical popup + / % / −, highlighted when not 100%; floating terminal handle and Preview toolbar − % + removed
- [x] 3.4 Files/Network/SFTP/Trash/Media bodies drawn with `Tokens::scaled` + `scale_style`; UI tests (header for every kind, larger text at 200%), snapshot `pane_zoom_popup_{dark,light}`, refreshed header snapshots
- [x] 3.5 CHANGELOG, devlog, docs/development.md; clippy, tests, layers, brand-check, fmt
- [x] 3.6 Icon: magnifier with a plus (zoom, not search)
- [x] 3.7 WCAG 2.2 AA (and no layout shift: fixed-width button, icon fixed at the right): percentage shown as text while zoomed + outline (1.4.1); 24×24 targets, header row ≥24 (2.5.8); contrast test for every accent (1.4.3/1.4.11); names "Zoom"/"Zoom, N%", "Zoom … in/out", expanded state (1.1.1/2.5.3/4.1.2); keyboard test Enter/Tab/Space/Escape with focus return (2.1.1/2.4.3)
- [x] 3.9 Media pane zoom includes the player: `Tokens::zoom`, icon buttons sized and icons drawn scaled (`Pen` + `Shape::transform`), seek/volume bars, clock/title, cover, playlist offsets, lock and row marks; test (buttons ×2, mutation-checked), snapshot `media_zoomed_{dark,light}`
- [x] 3.8 Manual: screen reader check (Orca on Linux, Narrator on Windows, VoiceOver on macOS) of the zoom button and popup (passed, user check 2026-11-16)
