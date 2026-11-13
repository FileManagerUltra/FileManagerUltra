## Why

Preferences cannot be used comfortably with the keyboard alone. Tab never reaches the topic list once you are in a topic, leaves the dialog into the panes behind it, and visits every option of a radio row separately. Focus is hard to see, a focused control can sit outside the visible area, the two dropdowns in Subtitles cannot be operated from the keyboard, and the File types lists trap Tab between them. Keyboard-only and low-vision users need a predictable path through the whole dialog.

## What Changes

- **Tab cycle stays in the dialog:** topic list → the controls of the current topic in reading order → Close → back to the topic list; Shift+Tab goes the other way. The window stays non-modal for the mouse (panes behind remain clickable).
- **Topic list is one Tab stop:** Up/Down/Home/End select a topic directly; Ctrl+Tab / Ctrl+Shift+Tab and Ctrl+PageDown / Ctrl+PageUp switch topics from anywhere in the dialog. Opening Preferences puts the keyboard on the topic list.
- **Choice rows are one Tab stop:** theme, density, size units, date format, shell, media opening, accent and subtitle colour swatches, subtitle font and edge (the two dropdowns become choice rows). Arrow keys move the choice, which applies at once like a click.
- **Visible focus:** a clear focus ring in the accent colour around the focused control while the keyboard is in use, in both themes; the focused control is scrolled into view.
- **Escape** closes an open popup first, then leaves a text field, then closes the dialog.
- **File types:** Tab moves forward through the types list, the patterns list and on to Restore, and Shift+Tab back; the lists keep their arrow keys. Pattern actions have keys (Insert adds, Delete removes, F2 or Enter edits) so the pattern toolbar no longer adds Tab stops while patterns exist.
- Cleanup: shared keyboard helpers in one module instead of per-widget code.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `app-shell`: "Topic-based preferences" gains keyboard navigation, focus visibility and the choice-row behaviour; theme and shell stay radio-style choices.

## Impact

- `fm-ui`: new `focus_nav` module (Tab boundaries, roving focus, next-frame focus requests, focus ring, scroll to focus, choice rows); `preferences.rs`, `preferences_subtitles.rs`, `preview_filters.rs` adopt it.
- Tests: keyboard-only walks through the dialog in kittest (Tab cycle, topic keys, choice rows, Escape order, scroll into view), snapshots of the focus ring in both themes.
- No engine, command or dependency changes.
