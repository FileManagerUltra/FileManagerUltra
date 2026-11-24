## ADDED Requirements

### Requirement: Jobs can ask a yes/no question
A running file job MAY park on a yes/no question (such as extracting an archive with a decompression-bomb ratio). The question SHALL be shown as a dialog with Continue and Cancel; answering goes through `job.answer {job, go}`, which is enabled only while a job waits on a question and rejects malformed params. Cancel, Escape, closing the dialog or cancelling the job SHALL answer no, and the job then ends as cancelled.

#### Scenario: Declining a question
- **WHEN** an Extract job asks whether to continue and the user presses Cancel
- **THEN** `job.answer {go: false}` is dispatched, the job ends as Cancelled and nothing it would have written exists

#### Scenario: Left-out items in the Activity panel
- **WHEN** a finished job left items out
- **THEN** its card lists each one as "Left out <path>: <reason>" below any failures
