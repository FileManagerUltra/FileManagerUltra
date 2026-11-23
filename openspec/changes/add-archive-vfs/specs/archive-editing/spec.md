## ADDED Requirements

### Requirement: Changes inside an archive are staged
Inside a writable archive (zip, tar family, 7z), copying or moving files in, creating folders, renaming, deleting and saving an edited file SHALL change only a staged set of changes for that archive; the archive file SHALL NOT be written until Save. Listings, previews, Find and further operations SHALL show the archive with the staged changes applied. Deleting inside an archive SHALL NOT use the trash; the confirmation SHALL say the items are removed from the archive when it is saved.

#### Scenario: Copy into a zip
- **WHEN** the user copies `report.pdf` into `site.zip/docs` with F5
- **THEN** `report.pdf` appears in the listing of `site.zip/docs`, and `site.zip` on disk has the same size and modification time as before

#### Scenario: Delete inside a tar.gz
- **WHEN** the user deletes `www/old.html` inside `site.tar.gz`
- **THEN** the confirmation says it is removed from the archive when saved, and the file disappears from the listing

### Requirement: Unsaved archive changes are visible
A Files pane whose location is inside an archive with staged changes SHALL show a bar above the listing naming the archive and the number of changes, with Save and Discard buttons; the pane's tab SHALL show an unsaved marker. The bar SHALL appear in every pane showing that archive.

#### Scenario: Bar after an edit
- **WHEN** the user creates a folder inside `site.zip`
- **THEN** the pane shows "site.zip has 1 unsaved change · Save · Discard"

### Requirement: Save rewrites the archive safely
Save SHALL run as a file job with progress and cancel. It SHALL first check that the archive file's size and modification time still match the mounted version and SHALL stop with "The archive changed on disk since it was opened", offering Save as copy or Discard, when they do not. It SHALL write the new archive to a temporary file next to the original through the same backend, verify it by reading its index back, and only then replace the original by rename. Unchanged zip members SHALL be copied without recompression; an encrypted archive SHALL stay encrypted with the same password. If saving fails or is cancelled, the original SHALL be unchanged, the temporary file removed, and the staged changes kept.

#### Scenario: Save a zip with one added file
- **WHEN** the user presses Save after adding `report.pdf` to `site.zip`
- **THEN** a job saves the archive, `site.zip` afterwards contains the old entries and `report.pdf`, and the unsaved bar disappears

#### Scenario: Archive changed by another program
- **WHEN** another program rewrites `site.zip` after it was opened and the user presses Save
- **THEN** nothing is written and the user is offered Save as copy or Discard

#### Scenario: Cancelled save
- **WHEN** the user cancels a save of a 3 GiB `backup.tar.xz` halfway
- **THEN** the original file is unchanged, no temporary file remains, and the staged changes are still shown

### Requirement: Save as copy and Discard
"Save as copy" SHALL write the archive with its staged changes to a location chosen with the path picker and keep the original mounted with its staged changes cleared. Discard SHALL drop all staged changes for the archive after a confirmation and return every pane showing it to the archive's saved content.

#### Scenario: Discard
- **WHEN** the user presses Discard and confirms
- **THEN** the archive shows its contents as on disk and the unsaved bar disappears

### Requirement: Unsaved archive changes are never lost silently
Closing the last pane or tab showing an archive with staged changes, and quitting the app, SHALL ask Save, Don't save or Cancel per archive, through the same prompt as other unsaved documents. An archive with staged changes SHALL stay mounted while navigating elsewhere in the same tab, and the bar SHALL return when the pane comes back to it.

#### Scenario: Quit with unsaved changes
- **WHEN** `site.zip` has staged changes and the user quits
- **THEN** the app asks whether to save `site.zip` before quitting, and Cancel keeps the app open

### Requirement: Archive editing commands
`archive.save`, `archive.save_as`, `archive.discard` and `archive.changes` (the list of staged changes) SHALL be commands with ids, menu paths (File menu, enabled only when the focused pane is inside an archive with staged changes) and bad-parameter tests.

#### Scenario: Save with nothing to save
- **WHEN** `archive.save` is dispatched for an archive without staged changes or for a uri outside any archive
- **THEN** it returns `Err` and nothing is written
