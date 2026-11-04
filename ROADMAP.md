# Roadmap

Where FileManagerUltra is heading. Plans, not promises: order and scope can change.
Engineering detail lives in [docs/roadmap.md](docs/roadmap.md) and the OpenSpec changes
under `openspec/`.

## Now: 0.1.x (beta)

Stabilise the first public release.

- Windows and macOS smoke checks: live folder refresh (including mapped network
  drives), the installed Windows shortcut icon, and USB / VeraCrypt devices.
- Signed and notarized installers, so the OS stops warning on first start.
- Fix what early users report.

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
- **Preferences menu** with a pane-layout restore policy (yes / no / ask).
- **Automation**: CLI / control channel on top of the command registry.
- **Plugins.**

## Principles

- Never crash: failures become actionable errors.
- The UI never blocks on I/O.
- Security is tested, not assumed: no secrets on disk, hostile input everywhere.

Ideas and bug reports are welcome as GitHub issues.
