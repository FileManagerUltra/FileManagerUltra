## ADDED Requirements

### Requirement: Narrow toolbar and clean text copy
When the preview toolbar options do not fit the pane width they SHALL move into a menu button at the right. In text previews the line-number gutter SHALL NOT be selectable, and copying selected text SHALL yield the text without line numbers or extra blank lines. Ctrl+C/X/V in Preview and Trash panes SHALL act on selected text, not on files.

#### Scenario: Copy text
- **WHEN** the user selects lines in a text preview and copies
- **THEN** the clipboard holds exactly those lines
