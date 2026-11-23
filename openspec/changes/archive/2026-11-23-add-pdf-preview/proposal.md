## Why

A PDF in the Preview pane shows only file information today, so reading one, filling in
a form or signing a contract means leaving the file manager for another app. A
built-in viewer with form filling and signing, visual and digital, covers the
everyday PDF chores without becoming a PDF editor. It works the same on local,
SMB and SFTP files.

## What Changes

- **PDF view in Preview.** A pure-Rust renderer (`hayro`) shows PDFs page by page in
  the Preview pane instead of the metadata card. Continuous scrolling, page
  navigation, Fit width / Fit page / 100%, pane zoom, outline (bookmarks), internal
  links, and page colours (original, dark, high contrast) for low-vision reading.
  Text can be found with the existing pane-local Find and selected and copied.
  Password-protected PDFs open after a password prompt; the password is never stored.
- **PDF thumbnails.** In the folder thumbnail grid, PDFs show their first page instead
  of a name tile (local files only, like the other thumbnails).
- **Fill in forms.** AcroForm text, checkbox, radio, list and dropdown fields can be
  filled in place. Each field gets a regenerated appearance so the result looks right
  in every viewer. Read-only, required and maximum-length flags are respected; form
  scripts are never run; XFA-only forms show an explanation.
- **Visual signatures.** Draw, type or pick an image. Place the signature in a
  signature field or anywhere on a page, and move or resize it before saving. Saved
  signatures are optional, opt-in and owner-only.
- **Digital signatures.** Sign with a PKCS#12 certificate file (`.p12`/`.pfx`) as a
  PAdES baseline (B-B) signature. RSA and ECDSA P-256/P-384 keys, SHA-256 or
  stronger, all in pure Rust (RustCrypto). A visual appearance is optional. The
  certificate password is requested each time, held only for the signing operation,
  then cleared.
- **Signature verification.** Existing signatures are listed with signer, signing
  time and a status: valid, changed after signing (with later revisions shown), or
  broken. Weak algorithms are flagged. Identity is shown as "not checked against a
  trust list"; trust lists, revocation and timestamps are out of scope (offline only).
- **Saving that preserves the original.** Save writes a copy next to the file (e.g.
  `contract (signed).pdf`), and Save As picks a name with the in-app path picker.
  Overwrite is a separate, confirmed action. Every save appends an incremental update
  and is written atomically, so existing signatures stay intact. Unsaved changes keep
  the Preview on the document and are never discarded silently.
- **Out of scope.** Editing page content or text, page organizing, annotations other
  than signatures, running JavaScript, XFA filling, smart-card (PKCS#11) signing,
  timestamp servers, LTV and trust-list validation.
- Everything is a command (`preview.pdf.*`, `pdf.form.*`, `pdf.sign.*`, `pdf.save*`)
  with bad-parameter tests.

## Capabilities

### New Capabilities
- `pdf-view`: rendering, navigation, zoom and fit, outline, links, page colours, text Find/copy, password-protected documents and bounded rendering of hostile PDFs.
- `pdf-forms`: detecting and filling AcroForm fields, appearance generation, script/XFA handling, unsaved-change handling and saving (copy, Save As, overwrite; incremental and atomic).
- `pdf-signing`: visual signatures (draw/type/image, placement, saved signatures), PKCS#12 digital signing (PAdES B-B) and verification of existing signatures.

### Modified Capabilities
- `preview-pane`: PDFs move from metadata-only to a rendered view in "Supported previews"; "Preview follows a source Files pane" holds the document while it has unsaved changes; "Hostile files are bounded" and "Preview commands" gain the PDF limits and commands.
- `preview-thumbnails`: PDFs get a first-page thumbnail in the folder grid.
- `security`: PDFs are untrusted input (no scripts, launch actions, external fetches or embedded-file opening); certificate and document passwords and private keys never touch disk or logs; saved signatures are owner-only and opt-in.

## Impact

- New crate `crates/pdf` (`fm-pdf`, L1, no I/O, no egui): document model over bytes,
  page rendering, text extraction, forms, incremental writer, visual signature
  appearances, CMS signing and verification. Registered in `xtask/src/layers.rs`.
- `fm-engine`: a PDF render service (own small pool and LRU, not a job lane), per-pane
  PDF/form session state, saving on the job lanes, and new `pdf_view_cmds.rs`,
  `pdf_form_cmds.rs`, `pdf_sign_cmds.rs`. `fm-preview`: `Kind::Pdf` routes to the new
  view; PDF thumbnails. `fm-ui`: PDF view, form widgets overlay, signature pad and
  dialogs, signature panel, theme tokens (dark and light). `fm-app`: menu entries.
- New dependencies, all pure Rust and MIT/Apache: `hayro` (rendering), `lopdf`
  (object editing, incremental save), `cms`, `x509-cert`, `der`, `p12-keystore`,
  `sha2`, `p256`, `p384`, `ecdsa`, `signature`. `rsa` and `zeroize` are already in
  the tree. `THIRD-PARTY-NOTICES.md` is updated.
- Uses the in-app path picker (`add-path-picker`) for Save As, certificate and image
  selection.
- Docs: `docs/architecture.md` (PDF section), `docs/roadmap.md` (moves the roadmap
  idea into this change), `CHANGELOG.md`.
