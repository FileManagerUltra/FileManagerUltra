## ADDED Requirements

### Requirement: Preview panes can edit
A Preview pane SHALL be able to enter Edit mode as specified in `text-editor`. While in Edit mode, read-only preview commands that act on rendered content (`preview.toggle_pretty`, zoom of images, video commands) SHALL be disabled, `preview.toggle_wrap` SHALL control the editor's soft wrap, and Preview Find SHALL be replaced by the editor's find. A Preview pane in Edit mode SHALL NOT follow a source Files pane's selection while its buffer is dirty; with a clean buffer it SHALL follow as usual, and a file change SHALL trigger the autosave or warning rules of `text-editor`. Pane state saved in workspaces SHALL record the file, caret and scroll of an edited file but never its unsaved content.

#### Scenario: Dirty buffer pins the pane
- **WHEN** a Preview pane follows a Files pane, is in Edit mode with unsaved changes (autosave off), and the user selects another file
- **THEN** the pane keeps its buffer and the warning from `text-editor` is shown instead of switching

#### Scenario: Workspace restore
- **WHEN** a workspace saved with a Preview pane in Edit mode is restored
- **THEN** the pane reopens the same file in Edit mode at the same caret and scroll, loaded from disk

#### Scenario: Old state file
- **WHEN** a pane state saved before this change is loaded
- **THEN** the Preview pane opens in preview mode as before
