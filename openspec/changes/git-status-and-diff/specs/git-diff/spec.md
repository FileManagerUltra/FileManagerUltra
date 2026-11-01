## ADDED Requirements

### Requirement: Diff pane compares two sources side by side
`PaneKind::Diff` SHALL show two sources in aligned columns: each is a working-tree file, a file at
a revision (HEAD, index, branch, tag or commit), or any file on disk. Added, removed and changed
lines are highlighted; changed line pairs highlight the differing words. Scrolling is synced.

#### Scenario: Compare with HEAD
- **WHEN** `diff.open` runs for a modified tracked file with left = HEAD and right = working file
- **THEN** a Diff pane shows both versions aligned with changed lines highlighted

#### Scenario: Drop a file on a side
- **WHEN** a local file is dragged onto the left or right half of a Diff pane
- **THEN** `diff.side` replaces that side with the file

#### Scenario: Compare two arbitrary files
- **WHEN** the user picks "Compare with…" and selects another file
- **THEN** the pane shows the two files side by side

#### Scenario: Identical inputs
- **WHEN** both sources have identical content
- **THEN** the pane says the files are identical

### Requirement: Diff navigation and modes
`diff.next` / `diff.prev` SHALL jump between changes; `diff.swap` exchanges sides; `diff.mode`
switches side-by-side/unified (unified is the automatic fallback in narrow panes); an
ignore-line-endings option SHALL be available. Bad parameters return `Err` and change nothing.

#### Scenario: Line endings
- **WHEN** a side is a committed blob and the platform is Windows
- **THEN** line endings are ignored by default, since a CRLF checkout would otherwise differ on every line

#### Scenario: Next change
- **WHEN** `diff.next` runs
- **THEN** the view scrolls to the next change and the current change is indicated

### Requirement: Diff inputs are bounded
Each side SHALL be capped (default 2 MiB); larger inputs show a notice instead of a diff.
Binary inputs show "binary files differ" with sizes. Computation runs on a worker; the UI thread
never diffs, and results are dropped when the pane closes.

#### Scenario: Binary file
- **WHEN** a side is binary
- **THEN** no text diff is attempted and the pane reports binary files differ

#### Scenario: Missing revision or file
- **WHEN** a path does not exist at the chosen revision
- **THEN** that side is shown empty with an "added/removed" label, not an error
