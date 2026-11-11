## ADDED Requirements

### Requirement: Git status is shown for local repositories
For every Files pane whose folder lies inside a git working tree, the listing and the tree SHALL
show each entry's status as a colored name and a gutter status letter: Modified, Added,
Removed, Renamed, Untracked, Ignored, Conflicted. Clean entries show no marker. Status SHALL
come from the pure-Rust `gix` library; no `git` binary is required or invoked.

#### Scenario: Modified and untracked files
- **WHEN** a tracked file is edited and a new file is created in a repo folder shown by a pane
- **THEN** the edited file shows the Modified marker and the new file the Untracked marker without user action

#### Scenario: Folder aggregation
- **WHEN** a folder contains a modified file at any depth
- **THEN** the folder shows a "contains changes" marker; clean folders show none and ignored folders are dimmed

#### Scenario: Outside a repository
- **WHEN** a pane shows a folder not inside a repository, or an SFTP/SMB location
- **THEN** no gutter column or marker is shown and no error occurs

### Requirement: Line-ending conversion is not a change
A file that differs from the index only by CRLF line endings SHALL NOT be reported as modified when `core.autocrlf` is `true` or `input` (re-checked for at most 1 000 files per scan). On Windows the
system default `true` is assumed when the repository sets nothing.

#### Scenario: CRLF checkout
- **WHEN** `core.autocrlf=true` and a tracked file has CRLF endings but identical content
- **THEN** it shows no marker

### Requirement: Status never blocks and stays small
Repository discovery and status SHALL run on the `local` job lane, single-flight per repository,
debounced, and cached per repository (not per pane). Only changed entries are stored, capped at
200 000 with a visible "status incomplete" notice, and at most 8 repositories are cached.

#### Scenario: Large repository
- **WHEN** a pane shows a repository with 100 000 files
- **THEN** the UI stays responsive, the pane lists immediately and markers appear when status completes

#### Scenario: Unchanged repository is not rescanned
- **WHEN** focus returns and `.git/index` and `.git/HEAD` are unchanged
- **THEN** no status scan runs

### Requirement: Status stays current
Status SHALL refresh after live-watch invalidation of a visible repo folder, on focus regain and
on `git.refresh`, and the branch name SHALL be shown for the focused pane's repository.

#### Scenario: External commit
- **WHEN** changes are committed in a terminal
- **THEN** the markers clear within a few seconds and the branch name stays correct

### Requirement: Git integration is configurable
`prefs.git.enabled` (default true) SHALL disable discovery and markers entirely; colors and
letters SHALL be individually toggleable. Marker colors come from theme tokens in both themes.

#### Scenario: Disabled
- **WHEN** git integration is disabled
- **THEN** no repository is opened and no markers render
