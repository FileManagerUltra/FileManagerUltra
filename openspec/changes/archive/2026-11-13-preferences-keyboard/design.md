## Context

egui moves keyboard focus in widget registration order: Tab gives focus to the next widget that registers, and from the last widget of a frame it wraps to the first widget of the whole application (the panes drawn before the window). Shift+Tab from the first widget of the window lands on the last widget drawn before it. egui's own Tab handling can be suspended for a focused widget with a focus lock filter (`EventFilter { tab, … }`), and `Memory::move_focus` can restart it before a widget registers. egui draws no distinct focus ring for buttons (focus looks like "pressed") and does not scroll focused widgets into view. `ComboBox` has no keyboard navigation inside its popup.

## Goals / Non-Goals

**Goals:** a closed, predictable Tab cycle; roving focus for lists and choice rows; visible focus; Escape layering; testable in kittest without the eframe input hook.

**Non-Goals:** making Preferences modal for the mouse; changing settings, commands or the layout of topics; keyboard support in other dialogs (they can adopt the helpers later).

## Decisions

1. **Boundaries instead of a modal layer.** The first Tab stop (the selected topic) and the last (Close) lock Tab while focused and handle it: Tab on Close and Shift+Tab on the topic jump to the other end; the other direction restarts egui's order with `move_focus` before the widget registers. A modal layer would also trap focus but block clicks on the panes, which users rely on to see changes live.
2. **Next-frame focus requests by id.** Moving focus with arrow keys asks for a target id that is honoured from the next frame (a same-frame request would let the next widget handle the same key press again). Deterministic ids (`group.with(index)`) make targets known before they are drawn.
3. **Choice rows replace radio rows and the two Subtitles dropdowns.** One Tab stop (the selected option, the only focusable one), Left/Right/Up/Down/Home/End select and apply, accessible as radio buttons with a group label. Swatch rows use the same widget with painted swatches.
4. **Focus-visible ring.** Keyboard mode turns on with Tab, arrows, Home/End or Page keys and off with a pointer press; while on, the focused widget inside the dialog gets a 2 px accent ring painted above it. Text fields keep their own focus style.
5. **Scroll to focus** when a widget gains focus in keyboard mode: the settings or topic scroll area scrolls the minimum needed.
6. **Escape layering** uses the previous frame's state (egui clears focus and popups on Escape before widgets run): popup open → close it; text field focused → leave it; otherwise close the dialog.

## Risks / Trade-offs

- [egui internals change] → the behaviour is covered by kittest walks that fail if Tab escapes or skips.
- [Choice rows take more width than dropdowns] → they wrap (`horizontal_wrapped`).
- [Pattern toolbar out of the Tab order while patterns exist] → keys and tooltips cover its actions; with no patterns, the + button stays reachable.

## Migration Plan

None (UI only).

## Open Questions

None.
