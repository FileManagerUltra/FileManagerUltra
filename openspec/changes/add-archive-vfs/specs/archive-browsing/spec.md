## ADDED Requirements

### Requirement: Archives open like folders
Activating (Enter or double-click) a file recognised as an archive SHALL browse into it in the same pane, showing its root as a folder listing; the tree SHALL expand archives like folders. Back, Backspace and Up from an archive's root SHALL return to the folder holding the archive. A preference (Preferences › Archives › "Enter on an archive": Browse inside / Open with app, default Browse inside) SHALL select the activation behaviour, and the context menu SHALL always offer both "Browse inside" and "Open with default app".

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
An archive SHALL be recognised by case-insensitive file-name patterns (defaults: `*.zip`, `*.jar`, `*.tar`, `*.tar.gz`, `*.tgz`, `*.tar.bz2`, `*.tbz2`, `*.tar.xz`, `*.txz`, `*.tar.zst`, `*.tzst`, `*.7z`, `*.iso`, `*.rar`, `*.gz`, `*.bz2`, `*.xz`, `*.zst`) editable under Preferences › Archives, and SHALL be confirmed by its signature bytes when mounted. A file whose signature does not match its pattern SHALL show an actionable error naming the expected format instead of a listing.

#### Scenario: Wrong extension
- **WHEN** the user enters `notes.zip` whose content is plain text
- **THEN** the pane shows "notes.zip is not a zip archive" and stays usable

### Requirement: Supported formats
The app SHALL read zip (including zip64, Deflate, Deflate64, Bzip2, LZMA, XZ, Zstandard and PPMd members), tar (ustar, GNU and PAX) plain or compressed with gzip, bzip2, xz or Zstandard, 7z (LZMA, LZMA2, BZip2, Deflate, PPMd), ISO 9660 with Joliet and Rock Ridge names, single-file gzip/bzip2/xz/Zstandard files (shown as a folder holding one file named without the compression suffix) and RAR 4 and 5 through an installed tool. Zip, the tar family and 7z SHALL be writable; ISO, RAR and single-file compressed files SHALL be read-only, and write commands SHALL be disabled inside them. All decoders SHALL be pure Rust except the RAR tool.

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

### Requirement: Encrypted archives ask for a password
Reading a member of an encrypted zip or 7z, or mounting a 7z with encrypted headers, SHALL ask for the password in an in-app dialog. The password SHALL be kept in memory only while the archive is mounted, SHALL be cleared on unmount, and SHALL never be stored, logged or shown. A wrong password SHALL show "Wrong password" and ask again; after three failures the read stops.

#### Scenario: Wrong then right password
- **WHEN** the user previews a file inside an AES zip, enters a wrong password and then the right one
- **THEN** the first attempt shows "Wrong password" and the second shows the file

### Requirement: RAR through an installed tool
RAR archives SHALL be listed and read through an installed 7-Zip (`7zz`, `7z`, `7za`, or `7z.exe` in the standard Windows location) or `unrar`, found on `PATH` or set in Preferences › Archives. When no tool is found, entering a RAR SHALL show which tool to install for the current OS. Remote RAR files SHALL be copied to the private temporary folder (at most 4 GiB) before the tool reads them. RAR archives SHALL be read-only.

#### Scenario: No tool installed
- **WHEN** neither 7-Zip nor unrar is installed and the user enters `data.rar`
- **THEN** the pane says "RAR archives need 7-Zip or unrar" with the install hint, and nothing is run

#### Scenario: Listing through 7-Zip
- **WHEN** `7z` is installed and the user enters `data.rar`
- **THEN** the pane lists the RAR's entries with sizes and dates

### Requirement: Archive preview
The Preview pane SHALL show, for an archive file, its format, compression method(s), entry count, packed and unpacked size, whether it is encrypted, the number of shadowed duplicate entries if any, and its top-level entries. Files inside an archive SHALL preview like files anywhere else.

#### Scenario: Preview of a 7z
- **WHEN** the Files pane cursor is on `tools.7z`
- **THEN** the Preview shows "7z · LZMA2 · 1 204 files · 38 MB → 112 MB" and its top-level folders

### Requirement: Archive browsing commands
`archive.enter`, `archive.unmount` and `archive.password` SHALL be commands with ids, menu paths and bad-parameter tests (malformed params return `Err` without changing state). `archive.enter` SHALL accept a file uri and SHALL fail with an actionable error for non-archives.

#### Scenario: Malformed params
- **WHEN** `archive.enter` receives a directory uri, a 10 000-byte string or no uri
- **THEN** it returns `Err` and the pane is unchanged
