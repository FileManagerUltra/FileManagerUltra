## ADDED Requirements

### Requirement: Archives open like folders
Activating (Enter or double-click) a file recognised as an archive SHALL browse into it in the same pane, showing its root as a folder listing. The tree SHALL show every archive opened in the session (and any archives around it) as an expandable folder, sorted by name among the folders, until it is closed with "Close Archive" from its tree row, which SHALL also move panes inside it to the folder holding it; a preference ("Show all archives as folders in the tree", default off) SHALL make every archive file in the tree expandable like a folder. Back, Backspace and Up from an archive's root SHALL return to the folder holding the archive. A preference (Preferences › Archives › "Enter on an archive": Browse inside / Open with app, default Browse inside) SHALL select the activation behaviour, and the context menu's Archive submenu SHALL always offer both "Browse inside" and "Open with default app".

#### Scenario: Enter on a zip
- **WHEN** the cursor is on `site.zip` in a Files pane and the user presses Enter
- **THEN** the pane lists the root of the zip, and the breadcrumb ends with `site.zip`

#### Scenario: Leaving the archive
- **WHEN** the pane shows the root of `site.zip` and the user presses Backspace
- **THEN** the pane shows the folder holding `site.zip` with the cursor on it

#### Scenario: Old behaviour kept by preference
- **WHEN** "Enter on an archive" is set to Open with app and the user presses Enter on `site.zip`
- **THEN** the zip opens in its default application and the pane does not change

### Requirement: Archive recognition
An archive SHALL be recognised by case-insensitive file-name patterns (defaults: `*.zip`, `*.jar`, `*.tar`, `*.tar.gz`, `*.tgz`, `*.tar.bz2`, `*.tbz2`, `*.tar.xz`, `*.txz`, `*.tar.zst`, `*.tzst`, `*.7z`, `*.iso`, `*.rar`, `*.gz`, `*.bz2`, `*.xz`, `*.zst`, and for split archives `*.001`–`*.999` after an archive name, `*.z01`–`*.z99` and `*.r00`–`*.r99`) extendable under Preferences › Files › Archives with further name endings, each with the format to read it as (for example `cbz=zip`), and SHALL be confirmed by its signature bytes when mounted. A file whose signature does not match its pattern SHALL show an actionable error naming the expected format instead of a listing.

#### Scenario: Wrong extension
- **WHEN** the user enters `notes.zip` whose content is plain text
- **THEN** the pane shows "notes.zip is not a zip archive" and stays usable, offering Back and Open with Default App

#### Scenario: Added name ending
- **WHEN** the user adds `cbz=zip` under Preferences › Files › Archives and presses Enter on `comic.cbz`
- **THEN** the pane lists the pages inside it like any zip

### Requirement: Supported formats
The app SHALL read zip (including zip64, Deflate, Deflate64, Bzip2, LZMA, XZ and PPMd members; Zstandard members SHALL show "compression method not supported"), tar (ustar, GNU and PAX) plain or compressed with gzip, bzip2, xz or Zstandard, 7z (LZMA, LZMA2, BZip2, Deflate, PPMd), ISO 9660 with Joliet and Rock Ridge names, single-file gzip/bzip2/xz/Zstandard files (shown as a folder holding one file named without the compression suffix) and RAR 4 and 5 through an installed tool. Zip, the tar family and 7z SHALL be writable; ISO, RAR and single-file compressed files SHALL be read-only, and write commands SHALL be disabled inside them. All decoders SHALL be pure Rust except the RAR tool.

#### Scenario: Read-only ISO
- **WHEN** the pane shows the inside of `debian.iso`
- **THEN** files can be previewed and copied out, while New folder, Rename, Delete and pasting into the ISO are disabled with a tooltip saying ISO images are read-only

#### Scenario: Single compressed file
- **WHEN** the user enters `access.log.gz`
- **THEN** the pane lists one file `access.log` with its uncompressed size when known

### Requirement: Archives on every backend and nested
Archives SHALL open the same way on local, SMB and SFTP locations and inside other archives, up to 8 levels of nesting. Reading an archive on a remote location SHALL NOT require downloading it in full for seekable formats (zip, 7z, ISO, plain tar). Locations inside archives SHALL work in history, tabs, workspaces, bookmarks and the address bar, and SHALL be restored after a restart when the archive still exists.

#### Scenario: Zip inside a tar.gz on SFTP
- **WHEN** the user opens `backups.tar.gz` on an SFTP server and then `site.zip` inside it
- **THEN** the pane lists the zip's root and the breadcrumb reads `… › backups.tar.gz › site.zip`

#### Scenario: Restored after restart
- **WHEN** a workspace saved with a pane inside `site.zip/www` is reopened after a restart
- **THEN** the pane shows `site.zip/www` again; if `site.zip` was deleted meanwhile, the pane shows the location as unavailable without an error dialog

#### Scenario: Nesting limit
- **WHEN** the user tries to enter a ninth nested archive
- **THEN** the pane shows that the nesting limit was reached and the archive can still be copied out

### Requirement: Archives look different
Archive files, and every file and folder inside an archive, SHALL be drawn with the theme's archive colour tokens in the tree, the listing and the breadcrumb, in both dark and light themes. Archive files SHALL also carry an archive badge on their icon, and the breadcrumb SHALL show an archive icon on the archive segment, so colour is never the only cue. The tokens SHALL meet the theming contrast rules against the background and the selection colour.

#### Scenario: Inside an archive in the light theme
- **WHEN** the light theme is active and a pane shows the inside of `site.zip`
- **THEN** the entries use the archive colour, the breadcrumb's `site.zip` segment shows the archive icon, and the snapshot matches in dark and light

### Requirement: Opening big archives stays responsive
Mounting, indexing and member reads SHALL run off the UI thread on the archive job lane. Indexing a compressed tar SHALL show progress in the pane and SHALL be cancellable by leaving the location. A 100 000-entry local zip SHALL list within 2 seconds on the reference machine.

#### Scenario: Large tar.xz
- **WHEN** the user enters a 2 GiB `dump.tar.xz`
- **THEN** the pane shows "Reading dump.tar.xz… 35 %" while frames keep rendering, then the listing

### Requirement: Large archives list as they are read
A format that reads an archive from the start (compressed tars, RAR through a tool) SHALL make the entries it has read so far browsable while the rest is still being read: the folder being opened, and the tree, show them, and the pane says it is still reading and how many entries it has. When the read finishes the complete listing replaces the partial one; when it fails or is cancelled the pane shows the error. Leaving an archive that is still being opened (no pane is inside it any more, and a pane was while it was being opened) SHALL stop reading it. Formats whose index sits at the end (zip, 7z, ISO) list when complete. Copies of the partial listing SHALL be paced so that a big archive never costs a frame noticeable time.

#### Scenario: A big RAR set fills while it is read
- **WHEN** the user enters `backup.part01.rar` of 36 volumes and the tool has printed 5 000 entries
- **THEN** the pane lists the top-level entries found so far, with "Still reading backup.part01.rar… 5000 entries so far"

#### Scenario: The read fails half way
- **WHEN** the tool stops with an error after some entries were shown
- **THEN** the partial listing is replaced by the error

#### Scenario: Leaving an archive that is still being opened
- **WHEN** the user enters a large `dump.tar.gz` and, while it is still being read, goes to another folder and no other pane is inside it
- **THEN** reading it stops, and entering it again reads it anew; an archive only expanded in the folder tree (never entered) keeps being read

### Requirement: Encrypted archives open with a password
Encrypted zip (WinZip AES-128/192/256 and traditional ZipCrypto), 7z (AES-256, including encrypted file names) and RAR (through the tool, including encrypted file names) SHALL open after the user enters the password in an in-app dialog: when entering the archive if its file names are encrypted, otherwise when an encrypted member is first read. Encrypted entries SHALL carry a lock badge in the listing, and the archive preview SHALL name the encryption method. A password that opened one member SHALL be tried first for the others; a member it does not open SHALL ask again, keeping at most 8 passwords per archive. A wrong password SHALL show "Wrong password" and ask again; after three failures in a row the read stops. A CRC failure after a ZipCrypto read SHALL be reported as "Wrong password or damaged file".

#### Scenario: Wrong then right password
- **WHEN** the user previews a file inside an AES zip, enters a wrong password and then the right one
- **THEN** the first attempt shows "Wrong password" and the second shows the file

#### Scenario: 7z with encrypted names
- **WHEN** the user enters `secret.7z` whose headers are encrypted
- **THEN** the password dialog appears before any name is shown, and the listing appears after the right password

#### Scenario: Members with different passwords
- **WHEN** a zip holds `a.txt` encrypted with password A and `b.txt` with password B, and the user opens `a.txt` then `b.txt`
- **THEN** the dialog appears once for each password, and reopening either file afterwards does not ask again

#### Scenario: Forget password
- **WHEN** the user chooses "Forget password" for a mounted encrypted archive
- **THEN** the next read of an encrypted member asks for the password again

### Requirement: Split archives open as one archive
An archive split into parts SHALL open as one archive from any of its parts, on every backend, when all parts are in the same folder: numbered parts of any supported format (`name.ext.001`, `.002`, …), zip spanned sets (`name.z01`, …, `name.zip`) and RAR volumes (`name.part1.rar`, … or `name.rar`, `name.r00`, …). The set SHALL be found by name only, in the part's folder, with at most 10 000 parts. A missing part SHALL fail with a message naming it. Every part SHALL get the archive look in listings, and parts after the first SHALL show which part they are. Zip spanned sets and RAR volumes SHALL be read-only; numbered parts SHALL be as writable as their format.

#### Scenario: Enter the third part of a 7z
- **WHEN** the user presses Enter on `backup.7z.003` in a folder holding `backup.7z.001` to `backup.7z.005`
- **THEN** the pane shows the root of the whole archive, and the breadcrumb names `backup.7z.001`

#### Scenario: Missing part
- **WHEN** `backup.7z.004` is absent and the user enters `backup.7z.001`
- **THEN** the pane says "Part 4 of backup.7z is missing (backup.7z.004)"

#### Scenario: Spanned zip on SFTP
- **WHEN** the user enters `site.zip` on an SFTP server next to `site.z01` and `site.z02`
- **THEN** the pane lists the whole archive, and write commands are disabled with a tooltip saying spanned zip sets are read-only

#### Scenario: RAR volumes
- **WHEN** 7-Zip is installed and the user enters `data.part2.rar` next to `data.part1.rar` and `data.part3.rar`
- **THEN** the pane lists the whole archive

### Requirement: RAR through an installed tool
RAR archives SHALL be listed and read through an installed 7-Zip (`7zz`, `7z`, `7za`, or `7z.exe` in the standard Windows location) or `unrar`, found on `PATH` or set in Preferences › Files. When no tool is found, entering a RAR SHALL show which tool to install for the current OS. Remote RAR files SHALL be copied to the private temporary folder before the tool reads them. RAR archives SHALL be read-only.

#### Scenario: No tool installed
- **WHEN** neither 7-Zip nor unrar is installed and the user enters `data.rar`
- **THEN** the pane says "RAR archives need 7-Zip or unrar" with the install hint, and nothing is run

#### Scenario: Listing through 7-Zip
- **WHEN** `7z` is installed and the user enters `data.rar`
- **THEN** the pane lists the RAR's entries with sizes and dates

### Requirement: Archive preview
The Preview pane SHALL show, for an archive file, its format, compression method(s), entry count, packed and unpacked size, whether it is encrypted, the number of shadowed duplicate entries if any, and its top-level entries. Files inside an archive SHALL preview like files anywhere else. A gzipped log (a log name such as `app.log.2.gz` or `syslog.3.gz`, or a name the user's Preview file-type rules call a log) SHALL preview as a log, read compressed by the log viewer, instead of as an archive card; Enter still browses inside it.

#### Scenario: Preview of a 7z
- **WHEN** the Files pane cursor is on `tools.7z`
- **THEN** the Preview shows "7z · LZMA2 · 1 204 files · 38 MB → 112 MB" and its top-level folders

#### Scenario: Rotated log
- **WHEN** the Files pane cursor is on `/var/log/alternatives.log.2.gz`
- **THEN** the Preview shows the log's records, not an archive card

### Requirement: Archive browsing commands
`archive.enter`, `archive.unmount`, `archive.password` and `archive.password.forget` SHALL be commands with ids, menu paths and bad-parameter tests (malformed params return `Err` without changing state). `archive.enter` SHALL accept a file uri and SHALL fail with an actionable error for non-archives.

#### Scenario: Malformed params
- **WHEN** `archive.enter` receives a directory uri, a 10 000-byte string or no uri
- **THEN** it returns `Err` and the pane is unchanged

### Requirement: Temporary copies of archives share one limit
Temporary copies of archives (remote RAR sets before the tool reads them, unpacked compressed tars) SHALL be counted against one limit for all open archives together, set under Preferences › Files › Archives (default 4 GiB). A copy SHALL be made only when it fits in what is left of that limit and takes at most 80 % of the free space on the disk holding the temporary folder. A compressed tar SHALL be unpacked to a temporary copy from the second file read from it on, and read from the copy after that; when the copy does not fit, files SHALL still be read, decompressing from the start. Copies SHALL be deleted when their archive is closed or released.

#### Scenario: Two large archives
- **WHEN** the limit is 4 GiB, `a.tar.xz` already holds a 3 GiB copy and the user reads several files from `b.tar.gz`, which unpacks to 2 GiB
- **THEN** no copy is made for `b.tar.gz` and its files are still read correctly; after `a.tar.xz` is closed, the next reads from `b.tar.gz` make its copy

#### Scenario: Little free space
- **WHEN** the temporary folder's disk has 2 GiB free and a compressed tar unpacks to 1.8 GiB
- **THEN** no copy is made and reading stays slower
