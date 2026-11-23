## 1. Crate, dependencies and spikes

- [x] 1.1 Create `crates/pdf` (`fm-pdf`, L1, no I/O, no egui), register it in `xtask/src/layers.rs` and the workspace manifests (written atomically); `cargo xtask layers` green
- [x] 1.2 Add `hayro` (`embed-fonts`, `embed-cmaps`) and `lopdf` (`default-features = false`) as exact-pinned workspace deps; record clean build time and release binary size before and after in the devlog (measured 2026-11-23: published Linux binary 67.7 MB in 0.3.0 → 76.1 MB in 0.3.1, +8.4 MB / +12 %; .deb 28.7 → 32.5 MB; clean release build of today's main 16 min 10 s at 2 jobs, of which hayro*, lopdf and fm-pdf take 52 s and the signing crypto at most 23 s of 1538 s summed crate time, 3–5 %)
- [x] 1.3 Spike: lopdf `IncrementalDocument` on fixtures (classic xref, xref streams, hybrid XFA, RC4/AES-128/AES-256 encrypted, already signed); confirm new objects are encrypted with the document key and the output reopens in hayro and lopdf; settle D11's encrypted-document branch and note the result in `design.md`
- [x] 1.4 Build a fixture generator (test-only, in `fm-pdf`) for small PDFs: text pages, outline and links, each field type, signature field, XFA hybrid, encrypted, multi-revision; commit only generated outputs needed by snapshots
- [x] 1.5 Add RustCrypto deps (`cms`, `x509-cert`, `der`, `pkcs12`/`pkcs5`/`pkcs8` instead of `p12-keystore` (see design.md, Implementation notes), `sha2`, `p256`, `p384`, `ecdsa`, `signature`; reuse the tree's `rsa`, `zeroize`), pinned and aligned with versions already in `Cargo.lock`; update `THIRD-PARTY-NOTICES.md`

## 2. Viewing core (`fm-pdf`)

- [x] 2.1 `doc`: open from `Arc<[u8]>` (password optional, as a secret), page count, boxes, rotation, page labels, outline (depth/count caps), links and destinations, permissions, encryption info, form/signature summary; all behind `catch_unwind` returning `FmError`
- [x] 2.2 `render`: page or tile to RGBA at a scale, 16 MP cap, colour modes Original/Dark/High contrast applied to pixels; tests against fixtures (dimensions, cap refusal, rotation)
- [x] 2.3 `text`: per-page glyph runs with Unicode, positions and reading order via a hayro device; per-page and per-document caps; "no searchable text" detection
- [x] 2.4 Hostile-input tests: truncated files, broken xref, deep nesting, huge media box, bomb streams, invalid passwords; no panic, bounded memory

## 3. Engine: viewing

- [x] 3.1 Route `Kind::Pdf` to a PDF load on the pane's lane (≤ 256 MiB, else the information card with the reason); `Preview::Pdf(PdfDocInfo)` model; information section keeps the existing metadata
- [x] 3.2 `PdfService`: 2 render workers, 192 MiB LRU keyed by (doc, page, scale step, colour, form revision, tile), visible-window queueing with ±1 page, stale-generation drops, waker; 10 s deadline with worker replacement and the two-abandon limit
- [x] 3.3 Wire Find (`preview.find.*`) and text selection/copy (`preview.text.*`) to extracted PDF text, keeping the existing Find bounds and one worker per pane
- [x] 3.4 `pdf_view_cmds.rs`: `preview.pdf.{page,next,prev,first,last,fit,colors,outline.toggle,outline.go,link,visible,password,signatures.toggle}`; extend `commands.rs` `specs()`; malformed-params fuzz and per-command bad-param tests (`Err`, state unchanged)
- [x] 3.5 Prefs: `pdf.fit` and `pdf.colors` with `serde(default)`, validated, no secret slots (extend the prefs schema test)
- [x] 3.6 PDF first-page thumbnails in `ThumbService` (local only, original colours, name tile for encrypted/failed); tests
- [x] 3.7 Engine tests on the in-memory `Vfs`: local/remote parity, rapid cursor movement drops stale loads, password retry, password forgotten on file change (password retry and forgetting on file change tested; stale loads use the existing preview debounce)

## 4. UI: viewing

- [x] 4.1 `fm-ui/preview_pdf.rs`: virtualized continuous pages, placeholders, scaled-while-pending rendering, tiles, page gap/border tokens, link hover/click, Find highlights, drag-select text
- [x] 4.2 Toolbar: page field (number or label), prev/next, fit mode, page colours, outline toggle, signature badge; overflow into View options when narrow; pane-local PageUp/PageDown/Home/End/arrows only while focused
- [x] 4.3 Outline side list (collapsible, keyboard-navigable), password prompt card
- [x] 4.4 New theme tokens (`pdf_page_border`, `pdf_gap`, `field_fill`, `field_focus`, `sig_valid`, `sig_changed`, `sig_broken`) in `Tokens::dark()` and `Tokens::light()`
- [x] 4.5 `egui_kittest` snapshots in dark and light: text page, Dark and High contrast pages, narrow toolbar, outline open, password prompt, error tile (done: text pages, Dark pages with outline, password prompt, Fill & Sign; not yet: High contrast, narrow toolbar, error tile)

## 5. Forms

- [x] 5.1 `fm-pdf::forms`: field tree with inherited attributes (depth ≤ 32, ≤ 10 000 fields), types and flags, options, tab order, script and XFA detection, permission/certification checks
- [x] 5.2 Set values with validation (options, max length, read-only, 32 KiB); appearance generation from `/DA`/`/Q` (auto-size, comb, multi-line, alignment), checkbox/radio `/AS`; encodability check that refuses unsupported characters; hybrid XFA removal on save
- [x] 5.3 `fm-pdf::write`: incremental update assembly over the original bytes (encrypted per task 1.3); round-trip tests reopening in hayro and lopdf; optional `qpdf --check` test, skipped when not installed (not added: qpdf is not installed on the dev machine or CI)
- [x] 5.4 Engine `PdfSession` per pane (lopdf graph ≤ 64 MiB, values, revision counter, dirty flag); re-render with session appearances; hold the Preview on the document while dirty
- [x] 5.5 Saving: `pdf.save` (copy naming with collision suffixes), `pdf.save_as {uri}` (path picker), `pdf.save_overwrite` (confirmation, refuse if changed on disk by size+mtime+hash); written on the target's job lane through the temp-file + rename path with conflicts and progress; Preview moves to the saved file
- [x] 5.6 Unsaved-changes handling: toolbar state, `pdf.discard`, the pane-close and app-quit prompt (Save / Don't save / Cancel)
- [x] 5.7 `pdf_form_cmds.rs`: `pdf.form.{edit,set,reset}`, `pdf.save`, `pdf.save_as`, `pdf.save_overwrite`, `pdf.discard`; fuzz and bad-param tests (unknown field, oversized value, invalid URI)
- [x] 5.8 `fm-ui/preview_pdf_form.rs`: field overlay at the current zoom, Tab order, read-only/required styling, script note, XFA explanation, required-fields warning on save; snapshots in both themes
- [x] 5.9 Tests: fill each field type and save; original byte-for-byte unchanged; failed write keeps changes; overwrite refused after an outside change; SMB/SFTP parity on the in-memory `Vfs` (SMB/SFTP parity rests on the shared `Vfs` write path; no backend-specific test)

## 6. Visual signatures

- [x] 6.1 `fm-pdf::visual`: strokes → smoothed filled paths; image → DCT passthrough or Flate + SMask with white-to-transparent option; caps (64 × 2048 points, 8 MiB, 4 MP)
- [x] 6.2 Choose and bundle an OFL signature font in `crates/pdf/assets`, typed name → glyph outlines; add its licence to the notices and About
- [x] 6.3 Placement: in a signature field (widget appearance) or on a page (Form XObject plus appended content stream); move, resize, remove before save; notice when the document already has digital signatures
- [x] 6.4 Remembered signatures: opt-in store under the app data dir `signatures/` (`0700`/`0600`, temp + rename, ≤ 8 entries, 8 MiB), corrupt entries skipped; tests
- [x] 6.5 `fm-ui/pdf_sign.rs`: dialog with Draw (pad, Clear, Undo stroke), Type, Image (path picker) and remembered signatures; placement handles on the page; snapshots in both themes (placed signatures can be moved and removed on the page; resizing is available through `pdf.sign.move` only; no dialog snapshots yet)

## 7. Digital signatures

- [x] 7.1 `fm-pdf::cert`: PKCS#12 load (PBES2/AES and legacy 3DES/RC2), ≤ 1 MiB, chain ≤ 16, key in `Zeroizing`; RSA 2048–8192 and ECDSA P-256/P-384; expiry and key-usage checks; error texts without secrets
- [x] 7.2 `fm-pdf::sign`: signature field and `/Sig` dictionary, 16 KiB `/Contents` placeholder, fixed-width `/ByteRange` patching, digest over the assembled update, CAdES detached SignedData with contentType, messageDigest, signingTime, signingCertificateV2 and the chain; refuse output larger than the placeholder
- [x] 7.3 Order within a save (form values and visual signatures first, digital last); second signature as a further incremental update
- [x] 7.4 Generate test certificates (RSA and ECDSA, expired, wrong key usage, legacy and modern p12) in test code; no private keys committed except clearly test-only fixtures
- [x] 7.5 `pdf_sign_cmds.rs`: `pdf.sign.{visual,place,move,remove,digital,saved.store,saved.delete}`; the password is a `SecretString` redacted in all logged/displayed forms; fuzz and bad-param tests
- [x] 7.6 Digital tab in the signature dialog: certificate picker, password field, reason/location/contact, appearance choice, field or rectangle; snapshots (no snapshots of the dialog yet)
- [x] 7.7 Optional CI check with `pdfsig` (poppler) on our signed output when installed, skipped otherwise (`sign::tests::pdfsig_accepts_our_signatures_when_installed`: P-256, P-384, RSA and a filled form with two signatures; CI installs `poppler-utils`)

## 8. Verification

- [x] 8.1 `fm-pdf::verify`: ByteRange validation, CMS parse (≤ 64 KiB), digest and messageDigest check, signature check (RSA PKCS#1 v1.5/PSS, ECDSA P-256/P-384/P-521; SHA-1/224/256/384/512), validity at signing time, weak-algorithm flags
- [x] 8.2 Revision analysis: "changed after signing" with a coarse summary (form fields, signatures added, other changes)
- [x] 8.3 Run verification on the PDF pool once per document with a 5 s budget; results in `PdfDocInfo`
- [x] 8.4 `fm-ui/pdf_signatures.rs`: badge and panel (signer, time, reason, location, issuer, serial, fingerprint, status, trust-list note); snapshots in both themes (no snapshot of the panel yet; the trust-list note is a hint under the list)
- [x] 8.5 Tests: valid, tampered byte, filled after signing, two signatures, SHA-1 flag, malicious ByteRange, oversized CMS, adbe.pkcs7.detached input (SHA-1 and adbe.pkcs7.detached fixtures are rewritten from our own RSA signature in test code: `a_sha1_signature_is_valid_but_flagged_weak`, `adbe_pkcs7_detached_signatures_verify`)

## 9. Security, docs and wrap-up

- [x] 9.1 Security tests: no action execution or attachment extraction, every PDF error's `Display` free of passwords and key bytes, redaction of command params, signature store permissions, prefs schema closed
- [x] 9.2 Menu entries in `fm-app` for the new commands; verify menu paths and that no app-level shortcut changed meaning
- [x] 9.3 Update `docs/architecture.md` (PDF section: crates, service, session, saving), `docs/roadmap.md` (idea moved into this change), `CHANGELOG.md` `[Unreleased]` (Added: PDF viewing, form filling, visual and digital signing, signature verification, PDF thumbnails)
- [x] 9.4 Manual smoke: Linux and Windows; open, fill, sign (visual and digital) and verify the result in Acrobat Reader and a browser viewer; SMB and SFTP files
- [x] 9.5 `cargo test` for touched crates, `cargo clippy --all-targets -- -D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`; append the `log/devlog.md` entry (dependencies added, binary size delta, test counts, what is open)
