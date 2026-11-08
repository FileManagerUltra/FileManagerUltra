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

### Requirement: Jobs panel
The app SHALL show running and recently finished jobs (label, progress bar, cancel button, error/report summary) using theme tokens in both palettes.

#### Scenario: Visible progress
- **WHEN** a job is running
- **THEN** the status bar shows a count and the panel shows its bar and cancel button

#### Scenario: Failures stay visible
- **WHEN** a job finishes with failed items
- **THEN** the panel keeps the report with per-item errors until dismissed

### Requirement: Finished jobs refresh the view
On completion the engine SHALL invalidate the `DirCache` entries of every touched directory so affected panes re-read once.

#### Scenario: Listing updates after copy
- **WHEN** a copy into the other pane's directory finishes
- **THEN** that pane's listing shows the new items without manual refresh

### Requirement: Recent messages in the jobs panel
Command failures and warnings SHALL be kept in a bounded log of recent messages shown in the jobs panel, not in the status bar. A new message SHALL open the panel and close it again after 5 seconds unless the pointer is over it or the user opened it explicitly. An always-visible status-bar icon SHALL toggle the panel, and `messages.clear` SHALL empty the log.

#### Scenario: A failure appears in the panel
- **WHEN** a command fails
- **THEN** the jobs panel opens with the message and closes by itself after 5 seconds

#### Scenario: Messages stay reachable
- **WHEN** the panel has closed
- **THEN** clicking the status-bar icon shows the recent messages again
