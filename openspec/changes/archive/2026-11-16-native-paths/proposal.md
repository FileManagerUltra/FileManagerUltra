## Why

Locations reach the user in several forms depending on where they look: Find Files shows `file:///C:/Users` (and `file:///` for Search everywhere), an SMB bookmark is named `smb://192.168.0.50/`, the location bar shows SFTP folders as `user@host:/dir`, and hover texts show raw uris. On Windows none of these is how Windows writes a path. Worse, a text the app shows cannot always be typed back: the location bar's recent list sends a UNC path to Windows' own SMB access instead of the connection the folder came from. And clicking an SMB bookmark while that server is not connected fails with "no backend registered for this location" instead of signing in, unlike SFTP bookmarks.

## What Changes

- One display form per location, chosen by the OS the app runs on:
  - Local: `C:\Users\alice`, `\\server\share` and **This PC** on Windows; `/home/alice` elsewhere (unchanged).
  - SMB through the app's own connection: `\\host\share\dir` on Windows; `smb://host/share/dir` elsewhere (unchanged). An address with a user or a non-default port, which UNC cannot express, keeps the `smb://` form.
  - SFTP: `sftp://user@host/dir` on every OS. The location bar no longer uses the `user@host:/dir` form.
- Every place that shows a location uses that form: location bar, recent locations, Find Files (folder field, result folders, status line, recent searches), bookmark default names and hover texts, Properties, Copy Path, device labels.
- Every display form can be typed back. On Windows a typed or pasted `\\host\share\...` uses the app's own SMB connection when one to that host is open, otherwise Windows' own access (as today). A recent location picked from the list always returns to the place it was recorded from.
- The Find Files folder field accepts the same absolute forms as the location bar (`This PC`, `D:`, `~`, UNC paths, `smb://` and `sftp://` addresses).
- Opening an SMB location without a connection (bookmark, Places, recent location, Show in Folder) starts the sign-in, like SFTP already does, and lands there once connected.
- Stored addresses stay canonical uris (bookmarks, workspaces, recent searches, recent locations), so settings stay portable across OSes and never become ambiguous. A bookmark whose saved name is still its raw address is shown with the new default name.

## Capabilities

### New Capabilities
- `path-display`: how a location is shown and how typed text is turned back into a location, per OS.

### Modified Capabilities
- `navigation`: the location bar shows the display form; opening an unconnected SMB location signs in instead of failing.
- `file-search`: the folder field shows and accepts the display form; Search everywhere shows **This PC** on Windows and `/` elsewhere.
- `places`: bookmark default names use the display form; bookmarks on an unconnected server reconnect.

## Impact

`fm-vfs` (pure display formatter next to `native_path`), `fm-engine` (`item_cmds::display_path`, `goto_cmds::resolve` and recent locations, `nav.open`, `network::needs_login`, `search.start` root parsing, bookmark names), `fm-ui` (location bar label, Find Files folder field, hover texts). No new dependencies, no prefs format change. Not in scope: error message texts, which still name the canonical uri (follow-up), and the `network:///` / `servers:///` virtual folders.
