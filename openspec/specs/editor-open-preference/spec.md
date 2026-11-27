# editor-open-preference Specification

## Purpose
TBD - created by archiving change add-text-editor. Update Purpose after archive.
## Requirements
### Requirement: Internal or external editor per file kind
Preferences → Editor SHALL hold, per file kind (text, code, markup, config, log), a choice of *Internal editor* (the default) or *External editor (default app)*, set with `prefs.editor_open {kind, target}` where `kind` is one of those names or `all` and `target` is `internal` or `external`. Kinds SHALL be recognised with the same sniffing and filename filters as Preview. Activating an item with Enter or double-click in a Files pane SHALL keep opening it as before; F4 SHALL open it in Edit mode of a Preview pane, or in the default app when its kind is set to external. A kind set to *Internal editor* SHALL make Enter/double-click on that kind dispatch `editor.open`; set to *External* it SHALL dispatch `file.open` as before.

#### Scenario: Default behaviour
- **WHEN** a new user double-clicks `main.rs`
- **THEN** it opens in Edit mode in a Preview pane

#### Scenario: Prefer external for logs
- **WHEN** logs are set to *External editor* and the user presses F4 on `app.log`
- **THEN** `file.open` hands it to the default application

#### Scenario: Bad params
- **WHEN** `prefs.editor_open` receives an unknown `kind` or `target`, or a non-object
- **THEN** it returns `Err(InvalidParams)` and preferences are unchanged

### Requirement: The external editor is always available
`editor.open_external` SHALL open the focused item, or the file in the focused Preview pane in Edit mode, in the system default application whatever the preference. It SHALL appear in the context menu of editable files and in the Edit mode toolbar. Unsaved edits SHALL be saved first, or the buffer-warning shown when autosave is off, so the external program sees the current content. Remote files SHALL follow the existing copy-first behaviour of `file.open`.

#### Scenario: Hand off a dirty buffer
- **WHEN** the user runs Open in external editor on a dirty buffer with autosave on
- **THEN** the buffer is saved, then the file opens in the default application

#### Scenario: Preference is internal
- **WHEN** a kind is set to *Internal editor* and the user right-clicks a file of that kind
- **THEN** *Open in external editor* is still in the context menu

### Requirement: F4 opens a file for editing
F4 on the selected file of a Files pane SHALL run `editor.open {uri}`: it reuses the Preview pane following that Files pane if one exists, otherwise any Preview pane, otherwise splits a new Preview pane next to it; the item SHALL open in Edit mode there with focus in the editor. `editor.open` SHALL refuse directories and non-editable types with an actionable error, and SHALL accept an optional `line` to place the caret.

#### Scenario: Reuse the following Preview
- **WHEN** a Preview pane follows the Files pane and the user presses F4 on `config.toml`
- **THEN** that Preview pane switches to Edit mode on `config.toml` and takes focus

#### Scenario: No Preview pane
- **WHEN** only Files panes exist and the user presses F4
- **THEN** a Preview pane is created beside the focused pane and opens the file in Edit mode

#### Scenario: Directory
- **WHEN** F4 is pressed on a folder
- **THEN** an error says folders cannot be edited and no pane changes

