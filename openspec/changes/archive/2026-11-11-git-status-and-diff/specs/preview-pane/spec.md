## ADDED Requirements

### Requirement: Preview and Diff views
When the previewed file has a diff (see `git-diff`), the Preview toolbar SHALL show
**Preview** / **Diff** buttons that switch the view through `preview.diff.toggle {show}`. The
Diff view has a second bar with the revision button, "vs Working copy" and the diff options.

#### Scenario: Toggle
- **WHEN** the user clicks Preview while the diff is shown
- **THEN** the pane shows the normal preview and stays there for following files until Diff is chosen again
