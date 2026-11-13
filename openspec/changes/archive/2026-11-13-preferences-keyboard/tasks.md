## 1. Keyboard helpers

- [x] 1.1 `fm-ui/src/focus_nav.rs`: Tab boundary handling, next-frame focus requests by id, keyboard mode, focus ring, scroll-to-focus; unit tests
- [x] 1.2 Choice row widget (text options and painted swatches): one Tab stop, arrows select, radio semantics; tests

## 2. Preferences dialog

- [x] 2.1 Topic list: one Tab stop, Up/Down/Home/End, Ctrl+Tab / Ctrl+Shift+Tab / Ctrl+PageDown / Ctrl+PageUp, focus on open
- [x] 2.2 Closed Tab cycle: topic ↔ Close boundaries; focus ring and scroll-to-focus in the settings and topic areas
- [x] 2.3 Escape layering: popup, text field, dialog
- [x] 2.4 Replace radio rows, accent and subtitle swatches and the Subtitles dropdowns with choice rows
- [x] 2.5 File types: forward/backward Tab through types, patterns and onward; pattern toolbar out of the Tab order while patterns exist; Insert adds

## 3. Verification

- [x] 3.1 kittest keyboard walks: Tab/Shift+Tab cycle on General and a long topic, topic keys, choice rows, Escape order, scroll into view; snapshots of the focus ring in both themes; existing tests updated
- [x] 3.2 CHANGELOG, docs/development.md manual checks, devlog; clippy, fmt, layers, brand-check green
