# places Specification

## Purpose
Quick access to the user's standard folders and bookmarks from every Files pane and the Go menu.

## Requirements
### Requirement: Places lists standard folders and bookmarks
Every Files pane SHALL show two collapsible sections above the folder tree — Places (the user's existing standard folders: Home, Desktop, Documents, Downloads, Music, Pictures, Videos, each with its own icon) and Bookmarks (with a `+` button that bookmarks the current folder, and a hint when empty). The collapse state of each section SHALL be stored in the pane's layout state and persisted with the workspace; Go → Places SHALL list the same entries. Clicking an entry opens it in that pane (`nav.open`).

#### Scenario: Open a bookmark
- **WHEN** the user clicks a bookmark in Places
- **THEN** the pane navigates to it and the entry is highlighted while the pane shows it

### Requirement: Bookmarks are commands and are persisted safely
`bookmark.add {uri?, name?}` (Ctrl+D, default: the focused folder), `bookmark.remove {uri}`, `bookmark.rename {uri, name}` and `bookmark.move {uri, by | to}` (by ±1 or to an index; dragging a bookmark row reorders it) SHALL edit `Prefs.bookmarks`, which is saved by the owner-only prefs writer. At most 200 bookmarks of at most 100-character names and 4096-byte uris are kept; invalid entries in a hand-edited prefs file are dropped on load. Adding an existing bookmark is a no-op with a status message.

#### Scenario: Junk in the prefs file
- **WHEN** the prefs file contains a bookmark with an empty name or an unparsable uri
- **THEN** that bookmark is dropped on load and the others are kept

### Requirement: Places are drop targets
Dropping files on a Places or Bookmarks row SHALL copy/move them there, exactly like dropping on a folder-tree row.

#### Scenario: Collapse survives a restart
- **WHEN** the user collapses Bookmarks in a pane and restarts the app
- **THEN** that pane's Bookmarks section is still collapsed and the other panes are unaffected
