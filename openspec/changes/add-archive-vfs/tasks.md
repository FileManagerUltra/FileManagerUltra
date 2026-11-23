## 1. Crate, dependencies and spikes

- [x] 1.1 Create `crates/archive` (`fm-archive`, L1, no egui, I/O only through `Vfs`/`Source`), register it in `xtask/src/layers.rs` and the workspace manifests (written atomically); `cargo xtask layers` green
- [ ] 1.2 Add `zip` (default features off: `deflate`, `deflate64`, `bzip2`, `lzma`, `xz`, `zstd-rs`, `ppmd`, `aes-crypto`, `time`), `tar`, `bzip2` (pure-Rust backend), `lzma-rust2`, `ruzstd`, `sevenz-rust2`, `hadris-iso` (read features) as exact-pinned workspace deps; `cargo tree -e features` shows no `cc`/native `*-sys` builds; `cargo deny check` green; record clean build time and release binary size before and after in the devlog — deps added 2026-11-23 (zip without `zstd-rs`: it still builds `zstd-sys`; `hadris-iso` waits for 3.6); build time and binary size still to record
- [ ] 1.3 Spike: `sevenz-rust2` random member access in solid blocks and rewrite with an added file; `hadris-iso` read of Joliet + Rock Ridge fixtures; note results in `design.md`
- [ ] 1.4 Spike: `ruzstd` encoder output decodes with the reference decoder (test fixture) and its speed on 100 MB; decide whether tar.zst is writable in this change (D11) and note it in `design.md`
- [ ] 1.5 Spike: zip spanned sets (`.z01`… `.zip`) through a `ConcatSource` with the `zip` crate, or our own disk-offset translation; fallback via the 7-Zip tool; note the result in `design.md` (D16)
- [ ] 1.6 Spike: encryption round-trips — `zip` AES-128/192/256 read and AES-256 write, ZipCrypto read/write, `sevenz-rust2` AES-256 with and without encrypted headers; output opens in 7-Zip and Info-ZIP (fixtures)
- [ ] 1.7 Test fixture generator (test-only) for small archives of every format: nested folders, empty folders, symlinks, hard links, devices, unicode and hostile names, duplicates, encrypted variants (zip AES-128/192/256, ZipCrypto, mixed passwords per member; 7z AES with and without encrypted names; RAR with `-p` and `-hp`), split sets (numbered parts of zip/7z/tar.gz, zip spanned set, RAR `.partN.rar` and `.rNN` volumes, a set with a missing part), a nested zip in a tar.gz

## 2. Locations and display (`fm-vfs`)

- [x] 2.1 `Scheme::Archive` (`archive://<handle>/<path>`), handle = 16 hex digits of SHA-256 over the canonical container URI; parse/display round-trip and hostile-input tests
- [ ] 2.2 `ArchiveRef { handle, container }` stored beside archive URIs in history, tabs, workspaces and bookmarks (serde default, bounded); migration-free loading of old files
- [ ] 2.3 Display form and typed-path resolution (left to right, entering archive files, nesting cap 8) in `path-display`; Linux and Windows display tests — display form done (`archive_cmds::display`, used by `display_path`); typed-path resolution still open

## 3. Reading archives (`fm-archive`)

- [x] 3.1 `Source` trait (random access, size, mtime) and the shared 1 MiB-block / 64 MiB LRU cache (32 MiB LRU per mount instead of one shared 64 MiB)
- [x] 3.2 `Index` model and limits (1 000 000 entries, 4 096-byte names, depth 64), name normalisation, duplicate shadowing count; unit tests with hostile names
- [x] 3.3 zip reader (all listed methods, zip64, AES/ZipCrypto with password)
- [ ] 3.4 tar reader (ustar, GNU long names, PAX), plain and gz/bz2/xz/zst; one-pass index with progress callback; member read by skip; spool-to-temp after the second read (cap 4 GiB and 80 % of free space) — reader, progress, cancel and member reads done; spool-to-temp still open
- [x] 3.5 7z reader with per-block cache (≤ 256 MiB) and encrypted headers (one decoded block per read, not cached yet)
- [ ] 3.6 ISO reader (Joliet, Rock Ridge), read-only
- [x] 3.7 Single-file gz/bz2/xz/zst as a one-entry folder
- [x] 3.8 `ArchiveFs: Vfs` over index + source (+ overlay, section 5); every decoder call inside `catch_unwind`; tests on the in-memory `Vfs` for every format (writes still refused until section 5; decoders guarded by `catch_unwind`, also inside the streaming thread)
- [ ] 3.9 Split archives: name-based set resolution (numbered parts, zip spanned, RAR volumes; gaps, 10 000-part cap, folder-only), `ConcatSource` with random access across parts, spanned-zip offset mapping (per 1.6), `ArchiveRef` volume kind; tests on local and the in-memory `Vfs`, missing-part errors, hostile next-volume names
- [ ] 3.10 Encryption on read: AES and ZipCrypto zip members, 7z AES with encrypted headers, lock badges in the index, wrong-password detection (verifier, CRC), up to 8 passwords per mount tried in order; `Secret` zeroised on drop; redaction tests
- [ ] 3.11 Fuzz corpus (truncated, bit-flipped, oversized headers) for zip, tar, 7z, ISO: no panics, bounded memory

## 4. Engine: mounting and browsing

- [ ] 4.1 Mount table (handle → container, max 16 mounted, 10-minute idle unmount for clean mounts), backend registration per handle, re-mount on restore; unavailable-location state when the container is gone — `LazyArchive` per handle, process-wide handle table, re-registration from it; persistence across restarts, the 16-mount cap and idle unmount still open
- [x] 4.2 `JobLane::Archive` (2 workers) and `BulkArchive`; `lane_for`/`bulk_lane_for` (one thread per lane, like the other lanes)
- [ ] 4.3 Recognition patterns in prefs (`archives.patterns`, `archives.enter`: browse/open), signature check on mount with the "is not a … archive" error — `archives_browse` pref, Preferences checkbox and signature check done; editable patterns still open
- [ ] 4.4 `archive_cmds.rs`: `archive.enter`, `archive.unmount`, `archive.password`; Enter/double-click and tree expansion route archives to `archive.enter` according to the preference; context menu "Browse inside" / "Open with default app"; extend `commands.rs` `specs()`; malformed-params fuzz and per-command bad-param tests — commands, `file.open` routing, Up from the root, context menu done; tree expansion still open
- [ ] 4.5 Password flow: in-app prompt (on enter for encrypted names, on first encrypted read otherwise), three attempts, per-mount password list, `archive.password.forget`, zeroised on unmount/quit; redaction tests — password field in the listing, per-mount list, forget done; three-attempt limit and preview prompt still open
- [ ] 4.6 RAR helper (including multi-volume sets resolved first, and encrypted names via `-p`): tool discovery (PATH, Windows 7-Zip location, preference), listing and member read with argument arrays after `--`, timeout and output cap, remote copy to the private temp folder (≤ 4 GiB), parser tests on recorded `7z l -slt` and `unrar lt` output, hostile member names; "needs 7-Zip or unrar" state
- [ ] 4.7 Engine tests: local/SMB/SFTP parity on the in-memory `Vfs`, nested archive, restore after restart, nesting limit, compressed-tar progress and cancel by navigation

## 5. Staged editing and save

- [ ] 5.1 `Overlay`: added/replaced files spooled to the private temp folder, created folders, renames (subtree), deletions; merged `read_dir`/`stat`/`open_read`; size cap
- [ ] 5.2 Writes on `ArchiveFs` go to the overlay; read-only formats return `Unsupported("read-only archive")`; existing copy/move/delete/rename/new-folder/F4 jobs work unchanged (tests)
- [ ] 5.3 Delete inside an archive is staged, never trashed; confirmation text
- [ ] 5.4 Save writers: zip (raw copy of unchanged members, new members with the dominant method, encryption kept), tar family (recompress whole stream), 7z (recompress, same method and block size); verify by re-index; temp next to container; rename-replace; re-mount
- [ ] 5.5 Save job: changed-on-disk check (size + mtime), progress, cancel, failure leaves original and overlay intact and removes the temp file; tests for each, including remote via the in-memory `Vfs`
- [ ] 5.6 Password changes: `archive.password.set` (set/change/remove; AES-256 or chosen ZipCrypto; 7z encrypted names) as a staged change; Save re-encrypts or decrypts every member; unsaved bar text; tests that the result opens only with the new password (and in 7-Zip)
- [ ] 5.7 Saving numbered-part archives: temporary parts at the original part size, verify joined, backup-rename swap with rollback, leftover-backup detection and restore offer on mount, surplus parts removed last; failure-injection tests on the in-memory `Vfs`; zip spanned and RAR sets read-only
- [ ] 5.8 `archive.save`, `archive.save_as` (path picker), `archive.discard`, `archive.changes`; File menu wiring and enablement; bad-param tests
- [ ] 5.9 Dirty mounts: never unmounted; included in quit and pane/tab-close prompts (shared with PDF Fill & Sign and workspaces); tests

## 6. Extract and compress

- [ ] 6.1 `archive.extract` (Extract here with `name (2)` when taken; Extract to… with the path picker) on the copy pipeline; context menu and File menu
- [ ] 6.2 Extraction containment: links, hard links, devices/FIFOs, setuid/setgid, OS-invalid names; skipped entries in the job summary; tests with hostile fixtures
- [ ] 6.3 Bomb handling: declared-size overrun stops; >1 GiB at >1000:1 asks; out-of-space error; tests
- [ ] 6.4 `archive.compress` writers (zip with AES-256 or chosen ZipCrypto, tar.gz, tar.xz, 7z with AES-256 and optional encrypted names, tar.zst if 1.4 passed); levels; password refused for tar formats; conflict prompt; password redaction tests
- [ ] 6.5 Split output: `split { size }` writes `name.ext.001…` for every format (≥ 64 KiB, presets); output re-opens as one archive
- [ ] 6.6 `archive.join` for numbered parts (missing-part refusal, conflict prompt, progress, cancel); not offered for spanned zip or RAR
- [ ] 6.7 Moving out of writable archives stages the removal; read-only archives offer copy only
- [ ] 6.8 Adding a selection in one job: shared source walker (`focused_selection`/`parse_sources` incl. Shift ranges, Ctrl picks, Select all, Find Files results; recursive folders, empty folders, relative layout, links stored not followed); one progress/conflict/summary; tests with mixed selections of 1, 7 and 10 000 items
- [ ] 6.9 `archive.compress_quick` ("Add to name.zip/.7z", default name rules) and `archive.add` (existing archive: mount, password, stage with conflict prompt + apply to all, save in the same job; stage only when the archive already has staged changes); read-only targets refused; bad-param tests (empty sources, non-archive, read-only, target inside sources)
- [ ] 6.10 Drop target: dropping a selection on an archive file in a listing or the tree dispatches `archive.add`; drag-and-drop contract tests; context menu entries "Compress to…", "Add to name.zip", "Add to name.7z", "Add to archive…" for any selection

## 7. UI

- [ ] 7.1 Theme tokens `archive` / `archive_muted` in `Tokens::dark()` and `Tokens::light()` with contrast tests; tint in tree, listing and breadcrumb; archive badge on icons; archive icon in the breadcrumb
- [ ] 7.2 Unsaved bar (Save / Discard, change count) in every pane showing the archive; tab marker
- [ ] 7.3 Password dialog (with "Forget password" and the RAR process-list note), Set/Change/Remove password dialogs, Compress dialog (format, level, password twice with match indicator, AES/ZipCrypto choice marked weak, encrypt names, split size, name, folder), Preferences › Archives (patterns, Enter behaviour, RAR tool path, spool cap)
- [ ] 7.4 Pane states: indexing progress, "is not a … archive", "needs 7-Zip or unrar", nesting limit, read-only tooltips on disabled commands
- [ ] 7.5 Archive preview card in `fm-preview` + `fm-ui` (format, methods, counts, sizes, encryption, shadowed duplicates, top-level entries)
- [ ] 7.6 Listing hints: lock badge on encrypted entries, "part n of N" on later parts of split archives, all parts tinted
- [ ] 7.7 egui_kittest snapshots in dark and light: inside a zip, inside an encrypted zip (lock badges), split parts in a folder, unsaved bar (including "password will change on save"), Compress dialog with password and split, password dialog, archive preview; interaction tests for Enter/Backspace and Save/Discard

## 8. Docs, checks and wrap-up

- [ ] 8.1 `docs/architecture.md` section "Archives" (scheme, mount table, lanes, overlay, save); `docs/roadmap.md`; `THIRD-PARTY-NOTICES.md`
- [ ] 8.2 `CHANGELOG.md` entries under Unreleased (including the Enter behaviour change and how to restore it)
- [ ] 8.3 `cargo test` for touched crates, `cargo clippy --all-targets --all-features -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --check`, `cargo deny check`
- [ ] 8.4 Manual smoke on Linux and Windows: zip, tar.gz, 7z, ISO, RAR with and without 7-Zip, encrypted zip/7z/RAR (open, edit + save, set and remove password, results opened in 7-Zip), split 7z/zip/RAR sets (enter any part, missing part, save a split 7z, join), an archive on SMB and SFTP, nested archive, edit + save + reopen in another tool, quit prompt
- [ ] 8.5 Append the `log/devlog.md` entry (sizes, timings, test counts, open items)
