# Restore focus after closing a pane

When closing the focused pane, focus returns to the pane that was active before
the focused pane was created or selected, rather than always choosing the first
pane in painting order.

## Why

`Layout` already tracks the previous focused pane for commander-style switching,
but closing a pane currently selects the first remaining leaf. This is
surprising after splitting an active secondary pane and immediately closing
the new pane.

## Scope

- Prefer the previous focused pane when the focused pane is closed.
- Keep current focus when closing a different pane.
- If prior focus is unavailable or is the pane being closed, select a surviving
  pane and keep focus bookkeeping valid.
