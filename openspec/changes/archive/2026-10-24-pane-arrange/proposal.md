## Why

Panes can be split, closed and Tab-swapped, but the arrangement is otherwise fixed: you cannot move a pane to another place in the tree, and resizing is limited to dragging a divider with the mouse. Terminator-style layouts (a pane dragged to any edge of any other pane, equalised grids, a temporarily zoomed pane) make the multi-pane window usable for real work, especially now that panes can be terminals.

## What Changes

- **Drag a pane to rearrange it**: dragging a pane's border shows five drop targets over the pane under the pointer — left, right, top, bottom (the dragged pane is re-inserted there as a new split) and centre (the two panes swap). Per-pane state, running terminals and focus travel with the pane.
- **Better resizing**: dividers highlight and show a resize cursor on hover, enforce a minimum pane size, and double-click resets that split to its natural proportion. Keyboard resize nudges the nearest divider in a direction.
- **Layout commands**: `pane.move {pane, target, edge}`, `pane.equalize`, `pane.zoom` (toggle: show only the focused pane, the others keep running), `pane.rotate` (flip the focused pane's parent split between side-by-side and stacked), `pane.resize.left|right|up|down {amount?}`.
- **Menu and shortcuts**: Panes ▸ Equalize, Zoom, Rotate, Resize ▸ ...; chords use `Ctrl+Shift+…` so they also work while a terminal pane is focused.

## Capabilities

### New Capabilities

- `pane-arrangement`: moving, equalising, zooming, rotating and resizing panes by mouse and command.

### Modified Capabilities

- `panes`: the split handle gains hover feedback, a minimum size and double-click reset (ADDED requirements; existing ones unchanged).

## Impact

- **Code:** `crates/layout` (`Edge`, `Layout::move_pane/equalize/rotate/resize`, `zoomed`), `crates/engine` (`arrange_cmds.rs`), `crates/ui` (`arrange.rs` drag overlay, header drag source in `pane.rs`), `apps/fm-app` (renderer honours zoom, divider widget, menu, shortcuts).
- **Dependencies:** none.
- **Risks:** tree surgery must never lose a pane or its state (property tests over random trees); zoom must not leave focus on a hidden pane; drag-and-drop of panes must not collide with file drag-and-drop payloads.
