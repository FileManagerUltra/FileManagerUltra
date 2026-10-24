# preview-thumbnails Specification

## Purpose
TBD - created by archiving change thumbnails-and-open. Update Purpose after archive.
## Requirements
### Requirement: Folders get a thumbnail grid
A Preview pane showing a directory SHALL offer a thumbnail grid (`preview.thumbs.toggle`): image files render a downscaled thumbnail, video files render their poster frame, and everything else renders a name tile. The grid SHALL come in three sizes (`preview.thumbs.size`: small 160 px, medium 256 px, large 512 px) and a per-tile hover label with the full name. Leaving the Preview pane kind or showing a non-directory body SHALL collapse the grid back to the renderer's default.

#### Scenario: A folder of images
- **WHEN** a Preview pane follows a Files pane into a folder of images and thumbnails are enabled
- **THEN** tiles appear for the images (and name tiles for files without a thumbnail), and toggling thumbnails off returns the directory card

#### Scenario: Size presets
- **WHEN** `preview.thumbs.size` runs with no params while the grid is visible
- **THEN** the preset advances small → medium → large → small and the visible tiles are re-requested at the new edge

### Requirement: Only the visible window is requested
The grid SHALL tell the engine which entry window is on screen (`preview.thumbs.visible {first, count}`) and the engine SHALL queue those entries plus at most one extra screenful after and before them, newest window superseding older requests for that pane. Request parameters are bounded (count ≤ 10 000).

#### Scenario: Scrolling a large folder
- **WHEN** the user scrolls from the first to the fortieth screen of a 10 000-image folder
- **THEN** thumbnails for entries that are no longer visible are not queued and stale requests for that pane are cancelled, so the UI never waits on off-screen decoding

### Requirement: Thumbnail work is off the UI thread
Thumbnails SHALL be produced by a dedicated small worker pool (never the file-job lanes and never the UI thread), with decode failures, panics and unsupported formats contained as a name tile. A newer request for the same pane supersedes an older one; a hidden or closed pane's work is cancelled.

#### Scenario: Fast scroll
- **WHEN** the user drags the grid scrollbar across a folder of 2 000 images
- **THEN** frames keep painting and only the tiles that come to rest are decoded (intermediate windows are cancelled or skipped)

### Requirement: Cache is bounded and regenerable
Thumbnails SHALL be cached in memory in a byte-capped LRU (192 MiB) and on disk as PNG files in the platform cache directory (`<cache>/thumbs`), trimmed to 256 MiB on first use, keyed by uri + mtime + size + edge and versioned so a format change invalidates old files. `preview.thumbs.clear_cache` SHALL drop both memory and disk copies and re-request the visible window. A stale mtime/size SHALL miss the cache rather than show an outdated thumbnail.

#### Scenario: Cache size cap
- **WHEN** the disk cache exceeds 256 MiB
- **THEN** it is trimmed to the cap on next use and the newest files are kept

#### Scenario: Edited file
- **WHEN** a cached image is modified on disk and previewed again
- **THEN** the thumbnail is regenerated (the cache key includes mtime and size)

### Requirement: Thumbnails are local-only
Only entries whose uri scheme is local `file://` SHALL be thumbnailed; a remote directory SHALL keep the summary card and not pull every file over the wire. Decoding SHALL reuse the Preview limits (file size, dimension checks) and never run on the UI thread.

#### Scenario: Remote folder
- **WHEN** a Preview pane shows a folder on a remote backend with thumbnails enabled
- **THEN** no thumbnails are downloaded and the pane stays responsive

### Requirement: The grid is keyboard-navigable
When the grid has keyboard focus, arrows move one tile / one row, PageUp/PageDown move a screenful, Home/End jump to the first/last tile; the focused tile is painted with the selection token and scrolling is minimal (only when the cursor leaves the viewport). Each command SHALL exist in the registry with bad-param tests.

#### Scenario: Arrow through the grid
- **WHEN** the grid holds focus and the user presses Right then Down
- **THEN** the focus ring moves to the next and then the next-row tile and the source Files pane's keyboard handling does not steal the keys

#### Scenario: Bad window params
- **WHEN** `preview.thumbs.visible` receives negative numbers or a count above 10 000
- **THEN** it returns `Err(InvalidParams)` and requests nothing

### Requirement: Thumbnail commands
`preview.thumbs.toggle`, `preview.thumbs.size`, `preview.thumbs.visible` and `preview.thumbs.clear_cache` SHALL be registered commands with ids, menu paths, params documentation and `enabled` predicates that are no-ops unless the focused pane is a Preview; the toggle and size accept an optional `pane`.

#### Scenario: No Preview pane
- **WHEN** thumbnails are toggled while the focused pane is a Files pane
- **THEN** the command is a no-op (or an actionable error) and the layout does not change

### Requirement: The grid selects and acts like the Files pane
Click SHALL select one thumbnail, Ctrl/Cmd+click toggle, Shift+click extend from the anchor, right-click select the item unless already selected, double-click or Enter open it, and Ctrl+A select all. Arrow, Page, Home and End moves SHALL select the cursor item (Shift extends). While the grid's pane has focus its selection SHALL be the target of the file commands (copy, move, rename, mkdir, newfile, trash, delete), with the previewed folder as the directory for new items and the grid's own source pane excluded as an "other pane" destination. Dragging SHALL carry the whole selection through the existing drag-and-drop contract.

#### Scenario: Delete from the grid
- **WHEN** thumbnails are selected and `Delete` or `F8` is pressed
- **THEN** `file.delete` runs on exactly those items

### Requirement: A folder preview follows changes inside it
When a file action finishes in, or a refresh is requested for, the previewed folder (or a folder below it in recursive mode) the preview SHALL re-render in place, keeping the grid's selection, cursor and scroll position.

#### Scenario: Create inside a previewed folder
- **WHEN** `file.mkdir` creates a folder inside the previewed folder
- **THEN** the preview's items include it without the cursor moving

