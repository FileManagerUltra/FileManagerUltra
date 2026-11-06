## 1. Document model (fm-preview)

- [x] 1.1 Add `MdImage` and `MdBlock::Images`; hoist paragraph images after their text, keep link, cap at 200, keep placeholders in headings/cells
- [x] 1.2 Update `plain_text` (alt text) and Find/Copy paths; update module docs
- [x] 1.3 Parser tests: hoisting, linked badge, caps, hostile URLs (huge, control chars)

## 2. Resolution and loading (fm-engine)

- [x] 2.1 Share relative-path resolution with `preview.open_link`; reject absolute, `file:`, `data:`, `//`, root escape
- [x] 2.2 Add non-secret `preview_remote_images` pref (default false, malformed → false)
- [x] 2.3 Add `preview.md_images.want` command (id, label, params doc, enabled, run) loading local images via the file's `Vfs` lane with 8 MiB / 100 MP limits and contained decoder panics
- [x] 2.4 Remote loader on a 2-worker `net` lane: http/https only, 10 s timeout, 3 revalidated redirects, streamed 8 MiB cap, no cookies/credentials, public-address-only custom resolver
- [x] 2.5 Per-pane LRU cache (256 MiB), dropped on file change; cancel pending jobs on change; refresh open previews when pref toggles
- [x] 2.6 Tests: local round-trip on tempdir, missing/escaping path, oversize, SVG, pref off makes no connection, loopback/private/redirect-to-internal refused (local test server), malformed params fuzz returns `Err`

## 3. UI (fm-ui, fm-app)

- [x] 3.1 Render `Images` rows: wrap, scale to width, magnification, hover alt/title, placeholder states using theme tokens
- [x] 3.2 Linked image click goes through `preview.open_link`
- [x] 3.3 Preferences > Preview toggle "Load remote images in Markdown" wired to the pref
- [x] 3.4 `egui_kittest` snapshots in dark and light (loaded, loading, blocked)

## 4. Docs and wrap-up

- [x] 4.1 Update `docs/architecture.md`, `CHANGELOG.md` (Added/Security note) and `log/devlog.md`
- [x] 4.2 Run `cargo test`, `clippy -D warnings`, `cargo xtask layers`, `brand-check`, `fmt --check` for touched crates
