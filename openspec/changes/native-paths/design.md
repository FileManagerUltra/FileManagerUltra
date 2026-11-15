## Context

Locations are `VfsUri`s (`file`, `smb`, `sftp`, `network`, `servers`). `fm_vfs::native_path` already turns a local path into OS form, and `item_cmds::display_path` uses it for `file` uris, but returns `uri.to_string()` for everything else. About 20 call sites use `display_path`; others print the uri directly (Find Files folder field, preview hover texts, the location bar's own `authority:path` form for remote panes). `goto_cmds::resolve` turns typed text into a uri; on Windows it sends `\\host\...` to Windows' own access unless the pane is an SMB/Network pane. Recent locations (`Session::path_history`) store display strings and are re-resolved when picked.

`nav.open` calls `needs_sftp_login_to` before `backends.require`, so SFTP locations sign in, but SMB ones fail in `require` before `ensure_read` (which would call `needs_login`) is reached.

## Goals / Non-Goals

**Goals:**
- One pure function decides the display form of any uri for a given OS; one function turns any display form back into the uri it came from.
- Every user-visible location goes through the first; every typed address through the second.
- SMB locations sign in from `nav.open` like SFTP ones.

**Non-Goals:**
- Error message texts (`FmError` Display) keep canonical uris; a later change can route them through the display form.
- `network:///` and `servers:///` keep their uri form.
- Storage formats do not change.
- The SMB backend keeps its own protocol code; showing `\\host\share` is only text (the `smb-backend` rule against native UNC access is unaffected).

## Decisions

1. **Formatter in `fm-vfs`, label in `fm-engine`.** `fm_vfs::display_form(uri, windows: bool) -> String` is pure and tested for both OS flavours on every platform (like `native_path(path, windows)`). `item_cmds::display_path(uri)` calls it with `cfg!(windows)` and keeps the `This PC` label for the Windows drive list. Callers keep using `display_path`, so the switch is one function body plus the call sites that bypassed it.
2. **UNC only for what UNC can say.** In-app SMB uris display as `\\host\share\dir` on Windows only when the authority has no user and no port; otherwise the `smb://` form, so the text still names the right connection.
3. **Resolve with a connection check.** `resolve(base, text)` becomes `resolve_with(base, text, windows, smb_connected: &dyn Fn(&str) -> bool)`, pure and testable for both OSes; `Session::resolve_typed(base, text)` supplies `cfg!(windows)` and "the app has an SMB backend for this host". Windows UNC → `smb://` when the host is connected or the pane is SMB/Network (kept), else the native `file` UNC. Off Windows UNC is always `smb://` (unchanged).
4. **Recent locations store uris.** `path_history: Vec<VfsUri>`; suggestions show `display_path`. On commit, text equal to the display form of a recent entry resolves to that entry's uri before falling back to `resolve_typed`, so a picked entry always returns where it came from. The completion separator follows the display form (`\` for local and UNC paths on Windows).
5. **`nav.goto.commit` hands `nav.open` the canonical uri** (`target.as_str()`), not the display text, so `nav.open`'s plain `VfsUri::parse` never sees a UNC that means an in-app SMB location. `nav.open`'s params stay canonical-or-raw-path (it is also the control-channel surface).
6. **SMB sign-in from `nav.open`.** Split `needs_login(uri)` into `needs_login_to(uri, navigate)`; `nav.open` calls it with `navigate = true` before `backends.require`, beside the SFTP check. The pane keeps its location until the connection lands (as for SFTP); the existing landing logic navigates on success and `abandon` handles cancel.
7. **Find Files root.** The UI shows `display_path(root)`; `search.start` parses `root` with `resolve_typed` against the focused location but rejects text that is not anchored (relative). `draft.synced_root` compares display strings.
8. **Bookmark names.** `default_name` falls back to `display_path` for roots (already the case; the formatter change gives `\\host` on Windows). `Session::places()` shows `default_name(uri)` when a saved name equals the canonical uri string; prefs are not rewritten.

## Risks / Trade-offs

- [A UNC path on Windows means different backends depending on what is connected] → It is deterministic per moment, matches the user's decision, and recent locations/bookmarks bypass it by storing uris.
- [SFTP location bar text gets longer (`sftp://` prefix)] → It is now one form everywhere and can be pasted back; the old `user@host:/dir` form could not.
- [Workspaces saved with display-string history] → `path_history` is session-only (not persisted), so no migration.
