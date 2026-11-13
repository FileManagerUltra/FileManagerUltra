# Roadmap

Where FileManagerUltra is heading. Plans, not promises: order and scope can change.
Engineering detail lives in [docs/roadmap.md](docs/roadmap.md) and the OpenSpec changes
under `openspec/`.

## Now: 0.2.x (beta)

Make the current release solid on every platform and close the gaps in the Media pane.

- Windows and macOS smoke checks: live folder refresh (including mapped network
  drives), USB / VeraCrypt devices, the ffmpeg download on Windows and the installed
  Windows shortcut icon.
- Signed and notarized installers, so the OS stops warning on first start.
- Picture-based subtitles (DVD VobSub, Blu-ray PGS) in the Media pane.
- Hardware video decoding, with a fallback to software decoding.
- Fix what early users report.

What is already done on `main` but not released yet is listed under `[Unreleased]` in
[CHANGELOG.md](CHANGELOG.md).

## Next

- **File permissions.** View and change permissions and ownership (`chmod`/`chown` on
  Unix, ACL basics on Windows), including recursive changes and a clear explanation
  when an operation is denied. Works on SFTP where the server allows it.
- **Simple text and Markdown editor.** Edit in a pane, with save, undo/redo, find and
  replace, syntax highlighting and a live Markdown preview. Safe writes (no data loss on
  a failed save), and a warning when the file changed on disk. Works on remote files too.
- **Better search.** Builds on the existing Find Files window: regular expressions,
  include/exclude filters, search on SFTP/SMB, results you can open at the matching
  line, and search-and-replace across files.

## Later

- **Batch rename** with a before/after preview and safe handling of collisions.
- **File synchronization**: compare two folders, review a plan of copies, updates and
  deletions, then run it (one-way, mirror, two-way; saved folder-pair profiles).
- **Archives**: browse and extract zip/tar as folders.
- **Pane-layout restore policy** at startup: yes / no / ask, set in Preferences.
- **Automation**: CLI / control channel on top of the command registry.
- **Plugins.**

## Principles

- Never crash: failures become actionable errors.
- The UI never blocks on I/O.
- Security is tested, not assumed: no secrets on disk, hostile input everywhere.

Ideas and bug reports are welcome as GitHub issues.
