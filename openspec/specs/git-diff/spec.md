# git-diff Specification

## Purpose
TBD - created by archiving change git-status-and-diff. Update Purpose after archive.
## Requirements
### Requirement: Preview pane shows a diff against a revision
A Preview pane showing a single local file inside a git repository SHALL offer a **Diff** view
next to the normal preview. The diff compares the working copy (right) with the file at a
revision (left, default `HEAD`) in aligned columns. Added, removed and changed lines are
highlighted, and changed line pairs highlight the differing words. Folders, larger multi-selections,
and files outside a repository have no revision Diff view.

#### Requirement note: file-vs-file comparison
When exactly two files are selected, the pane offers a **Diff** toggle that compares them directly
(no git needed, never auto-shown, no revision picker). **File > Compare With…** and the context menu
open an in-app file picker; the chosen pair stays pinned until the selection moves. Files may be
local or on SFTP/SMB; each side is size-capped and loaded on its backend's job lane.

#### Scenario: Modified file opens in Diff
- **WHEN** the previewed file is Modified, Added, Renamed or Conflicted and its preview is text
- **THEN** the pane shows the Diff view by default with changed lines highlighted

#### Scenario: Unmodified file opens in Preview
- **WHEN** the previewed file is clean
- **THEN** the pane shows the normal preview and `preview.diff.toggle {show:true}` switches to Diff

#### Scenario: Identical inputs
- **WHEN** the working copy matches the revision
- **THEN** the view says there are no changes against that revision

### Requirement: The view choice and revision follow the pane
The pane SHALL carry over the user's Preview/Diff choice, the chosen revision (within the same
repository), the layout mode and the ignore-line-endings option when the pane moves to another file.

#### Scenario: Next file keeps Diff and the branch
- **WHEN** the pane shows the diff against branch `topic` and the selection moves to another file in the same repository
- **THEN** that file is also shown as a diff against `topic`, even if it is unmodified

### Requirement: Revision picker
`preview.diff.picker` SHALL open a searchable, resizable list of HEAD, recent checkouts, local
branches, remote branches and tags (folders for `prefix/` names, current branch marked).
`preview.diff.rev {rev}` sets the revision; empty, oversized, `-`-prefixed or NUL-containing
revisions are rejected, and an unknown revision shows an error in the view.

#### Scenario: Choose a branch
- **WHEN** the user picks a branch in the picker
- **THEN** the picker closes and the diff reloads against that branch

### Requirement: Diff navigation and modes
`preview.diff.next` (F7) and `preview.diff.prev` (Shift+F7) SHALL move to the next or previous
change and centre it in the view when the content allows it (a change taller than the view
starts three lines from the top; the view never scrolls past either end). The current change is
marked with an accent bar down its left edge and an outline around the block, and the bar shows
"Change N of M". Find in a diff SHALL centre the current match's line the same way and mark that
line with a solid accent outline and bar (only the side holding the match in split view), so
the search position stays easy to follow; the Find row's Previous/Next buttons are labelled in
words, like the Preview's Find bar. F7/Shift+F7 keep their file meanings when no diff is shown.
`preview.diff.mode` switches side-by-side/unified (unified is the automatic fallback in narrow
panes), `preview.diff.eol` toggles ignoring line endings and `preview.diff.reload` reloads.
The options sit in the diff bar and fold into a menu when the pane is narrow. Bad parameters
return `Err` and change nothing.

#### Scenario: Next change
- **WHEN** `preview.diff.next` runs, including when there is only one change
- **THEN** the view scrolls so the change sits in the middle of the view and the change is marked

#### Scenario: Find in a diff
- **WHEN** Find has matches in a diff and the user moves to the next match
- **THEN** the match's line is centred and outlined, and the match itself keeps its current-match highlight

#### Scenario: Line endings
- **WHEN** the platform is Windows
- **THEN** line endings are ignored by default, since a CRLF checkout would otherwise differ on every line

#### Scenario: Linked horizontal scrolling
- **WHEN** long lines extend beyond either side of a split diff
- **THEN** one shared horizontal scrollbar and horizontal wheel/trackpad gestures over either side move both text columns by the same pixel offset
- **AND** line-number gutters, the split divider, row backgrounds and change markers remain fixed
- **AND** each side clips text to its own content area so it cannot cover its gutter or the other side
- **AND** resizing keeps the columns within the new pane width without feeding scrollable text width into the pane layout
- **AND** rows remain virtualized, unified mode can scroll long lines horizontally, and change navigation retains its vertical behavior

### Requirement: Diff inputs are bounded
Each side SHALL be capped (default 2 MiB); larger inputs show a notice instead of a diff.
Binary inputs show "binary files differ". Loading and diffing run on the local lane; the UI
thread never diffs, and results are dropped when the pane goes away or the file changes.

#### Scenario: Binary file
- **WHEN** a side is binary
- **THEN** no text diff is attempted and the view reports binary files differ

#### Scenario: Missing path at the revision
- **WHEN** the file does not exist at the chosen revision
- **THEN** that side is shown empty, not as an error

