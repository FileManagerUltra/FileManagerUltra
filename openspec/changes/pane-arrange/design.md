## Context

`fm-layout` holds a binary `PaneNode` tree (`Leaf` / `Split{axis, ratio, a, b}`) plus `states: BTreeMap<PaneId, PaneState>`. Topology changes are engine commands; `fm-app` renders the tree and writes ratios while dragging a divider.

## Decisions

**D1 — Move = detach + re-insert.** `PaneNode::detach_leaf(id)` removes a leaf, promotes its sibling (like `close_leaf`) but keeps the id's `PaneState`. `insert_beside(target, id, edge)` replaces `Leaf(target)` with `Split{axis, 0.5, ..}`; `Left`/`Top` put the moved pane in `a`. `Edge::Center` is a plain `swap`. Moving a pane onto itself, or onto a target that no longer exists, is an error and leaves the tree untouched (the operation is transactional: validate first, mutate second).

**D2 — Equalize by span.** `ratio = span(a)/(span(a)+span(b))` where `span` counts leaves along the split's axis (same-axis children add, cross-axis children take the max). A row of three panes becomes thirds, not 50/25/25.

**D3 — Zoom is view state on `Layout`** (`zoomed: Option<PaneId>`), not a tree change. The renderer shows only that pane; other panes (and their terminals) keep their state and shells. Zoom clears when the zoomed pane closes or moves, and focusing another pane (command or click) un-zooms to avoid keyboard focus on a hidden pane.

**D4 — Rotate** flips the axis of the parent split of the focused pane (a leaf-only tree is a no-op).

**D5 — Keyboard resize** moves the nearest ancestor divider whose axis matches the direction by a step (default 5 %, clamped to `MIN_RATIO..=MAX_RATIO`); with no matching ancestor it is a no-op.

**D6 — Drag payload.** A dedicated `PaneDrag(PaneId)` egui payload, distinct from file `DragPayload`, so file drop zones ignore it and pane drops ignore files. Source: the pane header (drag sense on the header frame; the breadcrumb and type menu keep their clicks). Target: `arrange::edge_for(rect, pos)` — outer 25 % band per side picks that edge, nearest edge when bands overlap, otherwise Centre. The overlay paints the half that the pane would occupy, with theme tokens only.

**D7 — Divider widget** lives in `fm-app` next to the recursive renderer: hover colour from `tokens.accent`, `CursorIcon::ResizeHorizontal/Vertical`, double-click → `set_ratio(handle, equalized ratio)`. Minimum pane size 80×60 px as today, so a too-small window degrades instead of overlapping.

## Risks / Trade-offs

- Tree surgery bugs lose panes → property test: any sequence of split/move/close/rotate/equalize keeps the leaf set equal to the key set of `states`, ratios in band, focus valid.
- `zoomed` could point at a removed pane → every mutating `Layout` method calls `fix_zoom()`.
- Egui drag over a terminal body: the terminal widget senses drags for selection, but a pane drag starts only from the header, so there is no conflict.
