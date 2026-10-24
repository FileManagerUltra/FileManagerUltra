## ADDED Requirements

### Requirement: Panes can be moved anywhere in the tree
`pane.move {pane, target, edge}` with `edge` in `left|right|top|bottom|center` SHALL re-insert `pane` relative to `target`: an edge splits `target` and places `pane` on that side with ratio 0.5; `center` swaps the two. The pane's state, terminal session and focus MUST travel with it, and no pane is ever lost or duplicated.

#### Scenario: Move to an edge
- **WHEN** panes A | B | C (A,B,C in a row) and `pane.move {pane: A, target: C, edge: "bottom"}` runs
- **THEN** the tree is B | (C over A), A keeps its location, selection and history

#### Scenario: Move to the centre swaps
- **WHEN** `pane.move {pane: A, target: B, edge: "center"}` runs
- **THEN** A and B exchange positions

#### Scenario: Invalid moves are rejected
- **WHEN** the pane equals the target, either id is unknown, the edge is unknown, or params are malformed
- **THEN** the command returns an error and the tree is unchanged

#### Scenario: Dragging a header
- **WHEN** the user drags a pane header over another pane
- **THEN** five targets (four edges, centre) are shown, the one under the pointer is highlighted with a preview of the resulting slot, and releasing dispatches `pane.move`
- **AND** Esc cancels without changes

### Requirement: Equalize
`pane.equalize` SHALL set every split ratio so panes along a split's axis get equal space (leaf-count weighted).

#### Scenario: Row of three
- **WHEN** three panes in a row with ratios 0.8/0.1 are equalised
- **THEN** each takes one third of the width

### Requirement: Zoom
`pane.zoom` SHALL toggle showing only the focused pane at full size. Other panes keep their state and running terminals. Closing or moving the zoomed pane, or focusing another pane, ends zoom.

#### Scenario: Zoom and restore
- **WHEN** `pane.zoom` runs twice
- **THEN** the layout is identical to before and no terminal was restarted

### Requirement: Rotate
`pane.rotate` SHALL flip the axis of the focused pane's parent split; with a single pane it is a no-op.

#### Scenario: Side-by-side to stacked
- **WHEN** two panes are side by side and `pane.rotate` runs
- **THEN** they are stacked top/bottom with the same ratio and order

### Requirement: Keyboard resize
`pane.resize.left|right|up|down {amount?}` SHALL move the nearest ancestor divider of the focused pane on that axis by `amount` (default 0.05, at most 0.5), clamped to the ratio band; without such a divider it is a no-op.

#### Scenario: Grow right
- **WHEN** the focused left pane of a two-pane row runs `pane.resize.right`
- **THEN** its ratio grows by 0.05

#### Scenario: Bad params
- **WHEN** `dir` is unknown or `amount` is not a finite number in range
- **THEN** it returns an error and nothing changes

### Requirement: Invariants hold under any sequence
For any sequence of `split`, `close`, `swap`, `move`, `rotate`, `equalize` and `resize` the leaf set MUST equal the set of pane states, every ratio MUST lie in `MIN_RATIO..=MAX_RATIO`, and focus and previous-focus MUST name existing panes.

#### Scenario: Randomised operations
- **WHEN** a long random operation sequence runs on a layout
- **THEN** the invariants hold after every step
