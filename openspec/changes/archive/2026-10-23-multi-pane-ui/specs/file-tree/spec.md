## ADDED Requirements

### Requirement: Each pane has a lazy directory tree
A pane's file tree SHALL show the uri hierarchy rooted at the pane's tree root; children of a node SHALL be fetched from the directory cache only when the node is expanded, and SHALL be released when it is collapsed.

#### Scenario: Collapsed nodes cost nothing
- **WHEN** a pane's tree is at its default state (root expanded, everything else collapsed)
- **THEN** only the root's children are held in memory; expanding another node triggers exactly one cached read for that node

#### Scenario: Expansion follows the cache
- **WHEN** the user expands a directory node whose listing is already cached
- **THEN** it renders immediately without a new job

#### Scenario: Read failure on expand
- **WHEN** expanding a node fails (permission denied)
- **THEN** the node shows an inline error affordance and stays expandable for retry

### Requirement: The tree is keyboard-navigable
The tree SHALL support arrow-key navigation between visible nodes, Right to expand or descend, Left to collapse or ascend, and Enter to open the focused node in the pane.

#### Scenario: Enter opens a directory
- **WHEN** the user focuses a directory node in the tree and presses Enter
- **THEN** the pane navigates to that directory (same as `nav.open`) and the listing updates

#### Scenario: Enter on a file
- **WHEN** the user presses Enter on a file node
- **THEN** nothing navigates (file activation is a later change) and no error occurs

### Requirement: The tree renders hostile names safely
Tree labels SHALL derive from cached entries without interpreting them as markup or paths, truncating display of absurdly long names while keeping the full name available (e.g. tooltip).

#### Scenario: A 4 KiB filename
- **WHEN** a cached directory contains a name of 4 KiB (the parser cap)
- **THEN** the tree renders an ellipsized label without layout blowup or panic, and the full name is recoverable from the row's tooltip/data

### Requirement: A refresh keeps the tree's place
Refreshing SHALL preserve the tree's expanded nodes, remembered focused node and scroll offset. An expanded node whose cache entry is missing SHALL be re-read (showing its loading state until the read lands, and the error affordance if it fails) rather than rendering as permanently empty; keyboard focus SHALL return to the remembered node once its row is back.

#### Scenario: Expanded subtree, focus and scroll survive a refresh
- **WHEN** the tree has expanded nodes, a remembered focused node and a non-zero scroll offset, and `nav.refresh` invalidates the cache
- **THEN** after the re-reads complete the same nodes are expanded, the same scroll offset is restored and keyboard focus returns to the remembered node

#### Scenario: The loading frames do not write back a collapsed offset
- **WHEN** the tree collapses to its loading root while the pane's directory re-reads
- **THEN** the stored scroll offset is left untouched (the clamped display offset of the gap is never persisted)
