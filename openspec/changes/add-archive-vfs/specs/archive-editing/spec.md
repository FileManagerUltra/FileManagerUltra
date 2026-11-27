## ADDED Requirements

### Requirement: Changes inside an archive are staged
Inside a writable archive (zip, tar family, 7z), copying or moving files in, creating folders, renaming, deleting and saving an edited file SHALL change only a staged set of changes for that archive; the archive file SHALL NOT be written until Save. Listings, previews, Find and further operations SHALL show the archive with the staged changes applied. Deleting inside an archive SHALL NOT use the trash; the confirmation SHALL say the items are removed from the archive when it is saved.

#### Scenario: Copy into a zip
- **WHEN** the user copies `report.pdf` into `site.zip/docs` with F5
- **THEN** `report.pdf` appears in the listing of `site.zip/docs`, and `site.zip` on disk has the same size and modification time as before

#### Scenario: Copy a multi-selection into an archive pane
- **WHEN** one pane shows `site.zip/docs` and the user selects 12 files and 3 folders in the other pane with Shift-click and presses F5
- **THEN** one copy job stages all of them, they appear in `site.zip/docs`, and the unsaved bar counts the change

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
Closing the last pane or tab showing an archive with staged changes, and quitting the app, SHALL ask Save, Don't save or Cancel, listing the archives in the same single prompt as other unsaved work (see app-shell: one prompt before anything is lost). An archive with staged changes SHALL stay mounted while navigating elsewhere in the same tab, and the bar SHALL return when the pane comes back to it.

#### Scenario: Quit with unsaved changes
- **WHEN** `site.zip` has staged changes and the user quits
- **THEN** the app asks whether to save `site.zip` before quitting, and Cancel keeps the app open

### Requirement: Setting, changing and removing a password
For a writable zip or 7z, "Set password…", "Change password…" and "Remove password" SHALL be staged changes applied on Save: every member is re-encrypted with the new password (zip: AES-256, or ZipCrypto only when chosen and marked as weak; 7z: AES-256, with encrypted file names by default) or decrypted. The new password SHALL be typed twice. The unsaved bar SHALL say that the password changes on save. Tar archives SHALL NOT offer these actions and SHALL say that tar has no encryption.

#### Scenario: Encrypt an existing zip
- **WHEN** the user sets a password on `site.zip` and presses Save
- **THEN** every member of the saved `site.zip` is AES-256 encrypted, and entering it again asks for the password when a file is opened

#### Scenario: Remove a password from a 7z
- **WHEN** the user removes the password from `secret.7z` (after entering it) and saves
- **THEN** the saved archive opens without a password and its file names are readable

### Requirement: Saving split archives
Saving an archive stored as numbered parts SHALL write it again as numbered parts of the original part size, verify the joined result, and replace the original parts so that a failure at any point leaves either the old set or the new set complete. Backups left by an interrupted save SHALL be detected on the next mount, which offers to restore them. Surplus old parts SHALL be removed only after the new set is in place. "Save as copy" of such an archive SHALL write numbered parts of the same part size, named by the archive the parts make up (`backup.7z.001`, …), with the same password and name encryption as a save in place.

#### Scenario: Save a copy of an encrypted split 7z
- **WHEN** the user edits `films.7z.001` (10 MB parts, encrypted with a password) and chooses Save as copy into another folder
- **THEN** that folder holds `films.7z.001`, `films.7z.002`, … of 10 MB each except the last, and the copy opens with the same password

#### Scenario: Add a file to a split 7z
- **WHEN** the user copies a 300 MB file into `backup.7z.001` (100 MB parts, 5 parts) and saves
- **THEN** the folder holds `backup.7z.001` to `backup.7z.008`, all of 100 MB except the last, and the archive opens with the new file

#### Scenario: Interrupted swap
- **WHEN** renaming the third new part fails during Save
- **THEN** the original five parts are restored, the staged changes are kept, and an error names the failed part

### Requirement: Archive editing commands
`archive.save`, `archive.save_as`, `archive.discard`, `archive.changes` (the list of staged changes) and `archive.password.set` (no password removes it) SHALL be commands with ids, menu paths (File menu, enabled only when the focused pane is inside an archive with staged changes) and bad-parameter tests.

#### Scenario: Save with nothing to save
- **WHEN** `archive.save` is dispatched for an archive without staged changes or for a uri outside any archive
- **THEN** it returns `Err` and nothing is written
