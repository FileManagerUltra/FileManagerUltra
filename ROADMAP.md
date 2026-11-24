# Roadmap

Where FileManagerUltra is heading. Plans, not promises: order and scope can change.
Engineering detail lives in [docs/roadmap.md](docs/roadmap.md) and the OpenSpec changes
under `openspec/`.

## Now: archives

Archives open like folders, on every backend. Done on `main` and listed under
`[Unreleased]` in [CHANGELOG.md](CHANGELOG.md): zip, tar (gz/bz2/xz/zst), 7z, ISO and
RAR (through an installed 7-Zip or unrar) browse like folders, also nested and on network
shares; encrypted archives ask for their password; split archives open as one; changes
are staged and written by Save, Save as copy or Discard, including saving split
archives back as parts of the same size; adding a selection to an archive (right-click "Add to archive…" or drop on the archive); moving files out of archives; Join Parts; an archive card in the Preview pane; Extract and Compress (also in parts, with
AES-256 passwords). Still to do before the release:

- Tab marker and the last pane states (RAR tool missing, nesting limit).
- A manual check on Linux and Windows (also with and without 7-Zip installed).

Alongside: fix what early users report, and the Windows/macOS checks that are still
open (installed Windows shortcut icon, macOS smoke checks).

## Next

- **Disk images and Docker as folders.** Read-only SquashFS, ext2/3/4, FAT, NTFS, JFFS2
  and CramFS images with their partitions, "Open as archive or image…" that finds
  archives inside any file, and local Docker/Podman containers, images and volumes.
- **Select with a rectangle.** Drag a rectangle over empty space in a Files pane to
  select the items it touches.
- **File permissions.** View and change permissions and ownership (`chmod`/`chown` on
  Unix, ACL basics on Windows), including recursive changes and a clear explanation
  when an operation is denied. Works on SFTP where the server allows it.
- **Simple text and Markdown editor.** Edit in a pane, with save, undo/redo, find and
  replace, syntax highlighting and a live Markdown preview. Safe writes (no data loss on
  a failed save), and a warning when the file changed on disk. Works on remote files too.
- **Better search.** Builds on the existing Find Files window (which already searches
  local folders, SFTP and SMB by name and content): regular expressions, include/exclude
  filters, results you can open at the matching line, and search-and-replace across files.
- **More previews.** Diagram previews for draw.io and Dia files.
- **Feature walkthrough.** A quick tour with balloons that shows off several features
  using built-in examples, so it never changes any of your files.

## Later

- **Batch rename** with a before/after preview and safe handling of collisions.
- **Pane-layout restore policy** at startup: yes / no / ask, set in Preferences.
- **Automation**: CLI / control channel on top of the command registry.
- **Plugins.**

## Done recently

- 0.3.x: Synchronize Folders, PDF fill-in and signing, CSV/TSV table view, signed and
  notarized macOS downloads.
- 0.2.x: Media pane (with picture subtitles and hardware decoding), Find Files redesign,
  Activity panel, per-pane zoom, diagnostic log.

## Principles

- Never crash: failures become actionable errors.
- The UI never blocks on I/O.
- Security is tested, not assumed: no secrets on disk, hostile input everywhere.

Ideas and bug reports are welcome as GitHub issues.
