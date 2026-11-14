## 1. Zoom

- [x] 1.1 `PaneState::term_scale` (serde default 1.0) with a bounded reader; bounds exported from fm-layout
- [x] 1.2 `term.zoom.in/out/reset {pane?}` in `term_zoom_cmds.rs`; tests (pane-local, bounds, prefs untouched, bad params, corrupt stored value)
- [x] 1.3 Terminal draws its grid with the scaled font; handle in the top-right corner (hover or not 100%), scrollback label below it; UI test clicking the handle
- [x] 1.4 Ctrl+Shift+= / - / 0 map to terminal zoom while a terminal has focus; test; shortcuts reference rows

## 2. Finish

- [x] 2.1 CHANGELOG, devlog; clippy, tests, layers, brand-check, fmt
