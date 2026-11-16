## Context

Places and Go → Places dispatch `nav.open` on the focused pane. For an unconnected server `nav.open` starts the sign-in with `navigate = true`; when the connection lands, `open_in_focused_files_pane` re-roots the focused pane's tree at the server unless the tree is already a Network/Servers tree. Network panes (`PaneKind::Network`, tree at `network:///`) and SFTP panes (`PaneKind::Sftp`, tree at `servers:///`) keep their tree for any host. `pane.kind.set` prepares a pane of either kind (`open_network`, `open_servers`).

## Goals / Non-Goals

**Goals:** remote places open in a pane of their kind; the Files pane is untouched.

**Non-Goals:** changing `nav.open` (typed addresses, Show in Folder and the tree still navigate the focused pane); remote locations reached by typing or from the tree.

## Decisions

1. **A separate command `place.open`** in a new module (`place_cmds.rs`), so `nav.open` keeps its contract.
2. **Target pane:** `from` = `pane` param or the focused pane. Wanted kind = Network for `smb`, Sftp for `sftp`, none otherwise. If `from` already has the wanted kind (or none is wanted) → `from`. Else the first pane of the wanted kind, else `layout.split(from, Horizontal, 0.5)` followed by `pane.kind.set {kind, pane}`.
3. **Then** focus the target and dispatch `nav.open {uri}`; the existing sign-in flow lands in the focused (target) pane, whose Network/Servers tree is kept.

## Risks / Trade-offs

- [The user clicks another pane while the sign-in dialog is open] → The connection lands in the focused pane, as it does today for any sign-in.
