## ADDED Requirements

### Requirement: One prompt before anything is lost
Quitting, closing a pane and closing a workspace tab SHALL ask at most one question. When the action would lose unsaved workspace changes, filled-in PDF fields or signatures, or staged archive changes, or (on quit) stop work still running, a single prompt SHALL list all of them, grouped by kind, with Save (save everything listed, then go on), Don't save (go on without saving) and Cancel. When only running work is affected the prompt SHALL offer Keep running and Quit anyway. Save SHALL go on only after every save it started has finished; a failed or skipped save SHALL cancel the quit or close, keeping everything. A remembered "Don't ask again" answer for workspaces SHALL still apply without listing them.

#### Scenario: Workspace, PDF and archive changes on quit
- **WHEN** a workspace, a filled-in PDF and `site.zip` all have unsaved changes and the user quits
- **THEN** one prompt lists the workspace, the PDF and `site.zip`, and Save saves all three before the app quits, without a second prompt

#### Scenario: Closing one of two panes on an archive
- **WHEN** two panes show `site.zip` with staged changes and the user closes one of them
- **THEN** nothing is asked; closing the second one asks about `site.zip`

#### Scenario: Only running work
- **WHEN** a copy is running and nothing is unsaved, and the user quits
- **THEN** one prompt lists the copy with Keep running and Quit anyway
