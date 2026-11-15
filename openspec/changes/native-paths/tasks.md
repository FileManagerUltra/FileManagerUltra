## 1. Display form

- [x] 1.1 `fm_vfs::display_form(uri, windows)` with tests for every row of the path-display table, both OS flavours
- [x] 1.2 `item_cmds::display_path` uses it (keeps `This PC`)
- [x] 1.3 Location bar label uses `display_path` for every scheme (drops the `authority:path` form)
- [x] 1.4 Hover texts and other spots that print `uri.to_string()` to the user (preview hovers, Go menu place hovers, subtitle/playlist file fields) use `display_path`

## 2. Typing it back

- [x] 2.1 `resolve_with(base, text, windows, smb_connected)` + `Session::resolve_typed`; Windows UNC → in-app SMB when connected; tests for both OSes
- [x] 2.2 `path_history` stores uris, suggestions show display forms, an unchanged recent entry commits to its own uri; completion separator follows the display form
- [x] 2.3 `nav.goto.commit` passes the canonical uri to `nav.open`
- [x] 2.4 `search.start` root goes through `resolve_absolute` (rejects relative text); folder field shows the display form (`This PC` / `/` for Search everywhere)
- [x] 2.5 Copy/Move destination dialog starts with the display form and resolves typed text like the location bar

## 3. SMB sign-in and bookmarks

- [x] 3.1 `needs_login_to(uri, navigate)`; `nav.open` signs in to an unconnected SMB location instead of failing; test that the login prompt opens and the pane is unchanged
- [x] 3.2 Bookmarks whose saved name equals their uri show the default name; test

## 4. Wrap-up

- [x] 4.1 CHANGELOG, devlog; `cargo test`/clippy/fmt/layers/brand-check for touched crates
- [ ] 4.2 Manual (Windows): location bar, Find Files (Ctrl+F, Ctrl+Shift+F), Copy Path and an SMB bookmark while disconnected
