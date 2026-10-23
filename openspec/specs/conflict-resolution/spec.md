# conflict-resolution Specification

## Purpose
TBD - created by archiving change dnd-and-file-ops. Update Purpose after archive.
## Requirements
### Requirement: Conflicts need an explicit decision
When a destination exists, the pipeline SHALL apply the job's `ConflictPolicy`; under `Ask` it SHALL park the worker, emit a conflict event with source and existing-destination metadata (name, size, mtime), and continue only after a decision. Nothing SHALL be overwritten without `Overwrite` being chosen.

#### Scenario: Ask parks the job
- **WHEN** a copy hits an existing file under `Ask`
- **THEN** the job emits `Conflict`, writes nothing for that item, and the UI thread is not blocked

#### Scenario: Overwrite decision
- **WHEN** the decision is Overwrite
- **THEN** the destination is replaced via temp-file-then-rename and the job continues

### Requirement: Decisions and apply-to-all
Decisions SHALL be Overwrite, Keep both, Skip or Cancel, each optionally "apply to all remaining conflicts" for that job, and SHALL be delivered through the `job.resolve_conflict` command.

#### Scenario: Apply to all
- **WHEN** the user chooses Skip with apply-to-all on the first of five conflicts
- **THEN** the remaining four are skipped without further prompts

#### Scenario: Cancel ends the job
- **WHEN** the decision is Cancel
- **THEN** the job stops, partial temp files are removed, and completed items remain

### Requirement: Keep-both naming
Keep both SHALL pick the first free `name (N).ext` (N from 2) by probing the destination, giving up with an error after a fixed cap.

#### Scenario: Next free name
- **WHEN** `a.txt` and `a (2).txt` exist and the incoming file is `a.txt`
- **THEN** it is written as `a (3).txt`

### Requirement: Unanswered conflicts never deadlock
A parked job SHALL observe cancellation, and a job whose decision channel is dropped SHALL end as cancelled.

#### Scenario: Cancel while parked
- **WHEN** `job.cancel` runs while a job waits on a conflict
- **THEN** the job ends cancelled promptly

