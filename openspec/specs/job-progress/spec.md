# job-progress Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
### Requirement: Jobs report progress
Mutating jobs SHALL emit events (`Progress { done, total, current }`, `Conflict`, `Finished(report)`, `Failed(err)`) through the existing job queue, and `Session::poll` SHALL expose running jobs with their latest progress without blocking.

#### Scenario: Progress is monotonic
- **WHEN** a multi-file copy runs
- **THEN** `done` never decreases and ends equal to `total` on success

#### Scenario: Poll never blocks
- **WHEN** a slow backend is mid-copy
- **THEN** `Session::poll` returns immediately with the last known progress

### Requirement: Jobs can be cancelled
`job.cancel` SHALL set the job's cooperative cancel flag; the pipeline SHALL check it between buffers and between items and finish with a cancelled report, leaving no temp files.

#### Scenario: Cancel mid-file
- **WHEN** a large copy is cancelled after some bytes
- **THEN** the temp destination is removed and the job ends `Cancelled`

### Requirement: Finished jobs refresh the view
On completion the engine SHALL invalidate the `DirCache` entries of every touched directory so affected panes re-read once.

#### Scenario: Listing updates after copy
- **WHEN** a copy into the other pane's directory finishes
- **THEN** that pane's listing shows the new items without manual refresh

### Requirement: Activity panel
The app SHALL show running and recently finished jobs (label, progress bar, cancel button, error/report summary) in a panel titled "Activity", using theme tokens in both palettes. The status-bar icon that toggles it SHALL have the tooltip "Activity: jobs and recent messages" and the accessibility label "Activity". User-facing text and docs SHALL call it the Activity panel.

#### Scenario: Visible progress
- **WHEN** a job is running
- **THEN** the status bar shows a count and the Activity panel shows its bar and cancel button

#### Scenario: Failures stay visible
- **WHEN** a job finishes with failed items
- **THEN** the panel keeps the report with per-item errors until dismissed

#### Scenario: New name
- **WHEN** the user hovers the status-bar icon or opens the panel
- **THEN** the tooltip and the panel title say "Activity", and no user-visible text still calls it "Jobs"

### Requirement: Recent messages in the Activity panel
Command failures and warnings SHALL be kept in a bounded log of recent messages shown in the Activity panel, not in the status bar. A new message SHALL open the panel and close it again after 5 seconds unless the pointer is over it or the user opened it explicitly. An always-visible status-bar icon SHALL toggle the panel, and `messages.clear` SHALL empty the log. A message MAY carry up to three actions, each a button label plus a command id with params. Clicking one SHALL dispatch that command through the registry, exactly as a menu item would, and failures SHALL be reported as ordinary messages. Action params SHALL be validated by the command, never trusted because they came from a message.

#### Scenario: A failure appears in the panel
- **WHEN** a command fails
- **THEN** the Activity panel opens with the message and closes by itself after 5 seconds

#### Scenario: Messages stay reachable
- **WHEN** the panel has closed
- **THEN** clicking the status-bar icon shows the recent messages again

#### Scenario: A message with actions
- **WHEN** a message with a "Play in Media pane" action is shown and the user clicks that button
- **THEN** the action's command is dispatched by id and the message's buttons no longer act a second time

