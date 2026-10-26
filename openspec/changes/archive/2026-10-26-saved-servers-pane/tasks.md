## 1. Model
- [x] 1.1 `Scheme::Servers` and `servers:///` URI (parse/display/round-trip, hostile input tests)
- [x] 1.2 Servers `Vfs` listing `Prefs.profiles` (SFTP only) as folders; capped and sanitised names

## 2. Engine
- [x] 2.1 Opening a saved server runs the connect flow (cached password silent, else popup); bad-params tests
- [x] 2.2 Connect popup state shared by Connect…, typed `sftp://` and "Open SFTP Pane"; Edit prefill
- [x] 2.3 Commands: `servers.edit`, reuse `remote.profile.save`/`delete`; graceful-failure tests

## 3. UI
- [x] 3.1 SFTP pane tree root `Saved servers`; header Connect… button
- [x] 3.2 Connect popup (replaces the inline form); context menu Connect/Edit/Delete
- [x] 3.3 Remove `connect_form`, `needs_connect` and the Back to Files path; update menu wiring
- [x] 3.4 Snapshots: Saved servers tree and popup, dark and light

## 4. Wrap-up
- [x] 4.1 Docs (architecture, development), devlog; `openspec validate --all`
- [x] 4.2 Per-crate clippy/tests; user runs `cargo xtask ci`; archive
