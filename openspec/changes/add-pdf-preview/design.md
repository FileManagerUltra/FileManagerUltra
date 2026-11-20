## Context

`fm-preview` sniffs `%PDF` as `Kind::Pdf` and renders a `MetaDoc` card ("PDF preview
shows file information only"). The Preview pane already has the machinery this change
builds on: per-pane `PreviewSlot`s with debounced, token-tagged renders on the pane's
lane; pane-local zoom (`preview.scale.*`) and Find (`preview.find.*`); model-backed
text selection; service pools outside the job lanes (thumbnails, Markdown images,
Mermaid) with LRUs and wakers; and the in-app path picker (`add-path-picker`).

Hard constraints (AGENTS.md): pure Rust, no `unsafe`, no FFI; the UI thread never
blocks; nothing below L4 names egui; every action is a command; hostile input
everywhere; secrets never on disk or in `Display`. Features must behave the same on
local, SMB and SFTP.

pdfcraft (MIT/Apache, `storytold/pdfcraft`) is the reference for behaviour and
structure: hayro for rendering, a separate writable object layer, incremental and
atomic saves, a forms crate that regenerates appearances, and a sign crate on
RustCrypto. We take its design, and small pieces of code where useful (with
attribution in `THIRD-PARTY-NOTICES.md`), but not its crates. They are git-only and
span about 35 crates plus craft-libs, and its RSA signing uses `aws-lc-rs` (C).

## Goals / Non-Goals

**Goals:**
- Read any PDF that renders in mainstream viewers, quickly, inside the Preview pane.
- Fill AcroForm fields and save a result that looks the same in Acrobat, pdf.js,
  PDFium and our own view.
- Visual signatures (draw/type/image) and PKCS#12 digital signatures (PAdES B-B).
- Verify existing signatures offline and say plainly what was and wasn't checked.
- Never damage the original: copy by default, incremental appends, atomic writes.
- Few dependencies, all pure Rust and MIT/Apache.

**Non-Goals:**
- Editing page content, text, images or page order; comments/annotations other than
  signatures; redaction; OCR; printing.
- JavaScript (calculate/format/validate/actions), XFA form filling.
- PKCS#11 smart cards, OS certificate stores, timestamp servers (TSA), LTV/DSS,
  trust lists (AATL/EUTL), OCSP/CRL revocation, certification (DocMDP) signatures.
- Fonts embedded for arbitrary scripts in filled fields (see D7).

## Decisions

**D1 Two libraries: `hayro` to read and render, `lopdf` to write.**
hayro (0.9, Apache/MIT) is read-only and lean, has the highest rendering correctness
of the Rust options (1,600+ PDF regression suite, pdfcraft's own oracle), and is
modular. It draws annotation appearances, which is how filled fields become visible.
Its device API exposes glyphs with Unicode and positions, enough for text extraction.
lopdf (0.47, MIT) parses into an editable object graph and has `IncrementalDocument`
for append-only updates. lopdf is used with `default-features = false` (no `rayon`,
no `chrono-clock`), hayro with `embed-fonts` (standard 14 fonts) and `embed-cmaps`
(CJK). *Alternatives:* pdfcraft crates by git (too heavy, moving fast, FFI in
signing); vendoring pdfcraft (a fork to maintain); one library for both (lopdf
cannot render; hayro cannot write); `pdf-writer` (writes new files, not incremental
updates of existing ones).
Viewing never loads lopdf. The writable graph is built only when the user starts
filling or signing, so plain viewing pays for one parse.

**D2 New crate `fm-pdf` (L1), pure and I/O-free.** Input is `Arc<[u8]>`, output is
models, pixels and new bytes. Modules: `doc` (open, page sizes, outline, links,
permissions, encryption), `render` (page or tile to RGBA), `text` (per-page glyph
runs for Find/copy), `forms` (field model, set values, appearances), `write`
(incremental update and byte assembly), `visual` (signature appearance streams),
`sign` (CMS build), `verify` (CMS/ByteRange checks), `cert` (PKCS#12 load).
hayro/lopdf/RustCrypto types never leave the crate; the public API is our own
structs. Every entry point runs inside `catch_unwind`, returns `FmError`, and checks
the caps in D10. *Alternative:* modules inside `fm-preview`. Rejected: signing and
writing are not "preview", and the crate boundary keeps the crypto deps out of
everything that only previews.

**D3 Engine: a `PdfService` plus per-pane `PdfSession`.**
- *Loading:* the file is read on the pane's lane (≤ 256 MiB, D10) into `Arc<[u8]>`,
  opened by `fm-pdf::doc`, and stored in the slot as `Preview::Pdf(PdfDocInfo)`
  (page count, page boxes and rotation, outline, permissions, form/signature
  summary). Remote files take the same path, so SMB/SFTP behave like local.
- *Rendering:* `PdfService` owns 2 render workers (not job lanes, so a large PDF
  doesn't queue behind copies) and a 192 MiB RGBA LRU keyed by
  `(doc id, page, scale step 0.25, colour mode, form revision, tile)`. The view
  reports visible pages (`preview.pdf.visible`); the service renders those, then
  one page before and one after, and drops stale generations, like `ThumbService`.
  Pages up to 16 MP render whole; above that, 1024 px tiles of the visible area.
  While a sharper render is pending, the previous scale is shown stretched.
- *Text:* extracted per page on demand (for Find, at most one worker per pane, as
  the existing Find requires), cached with the document, capped per page and
  document.
- *Editing:* `PdfSession` (one per Preview pane) holds the lopdf document, the
  field values as edited, placed signatures, a dirty flag and a revision counter.
  The view re-renders with appearances built from the session, so what you see is
  what will be saved.

**D4 Fit, zoom and page colours.** Fit width (default), Fit page and 100% are view
modes, and pane zoom (`preview.scale.*`, 50–400%) multiplies on top, following the
image convention (choosing a fit mode resets extra zoom). Colour modes are applied to
rendered pixels, not by changing the PDF: *original*; *dark* (luminance inverted,
hue kept, so photos stay recognizable); *high contrast* (thresholded to the theme's
text/background tokens). The mode is a per-pane view setting remembered in prefs;
the page border and gap colours come from theme tokens. This follows the
accessibility-first rule: one app-drawn path with the user's choice, the same for
every PDF.

**D5 Navigation and keys.** Continuous vertical scroll with virtualized pages. The
toolbar has a page field ("3 / 12", accepts page labels), previous/next buttons and
fit-mode and colour controls (folding into View options when narrow). Pane-local
keys only while the Preview has focus: PageUp/PageDown, Home/End and arrow scrolling,
which already mean "scroll this view" everywhere. No new app-level chords (shortcuts
are global). The outline opens as a collapsible side list in the pane. Internal links
(GoTo, named destinations) navigate; URI links show their target on hover and open
only through the existing explicit link-opening path used by Markdown; Launch,
GoToR, JavaScript and submit actions are ignored.

**D6 Forms: model and filling.**
- Fields come from `/AcroForm /Fields` (inherited attributes resolved, depth ≤ 32,
  ≤ 10 000 fields). Supported: text (single/multi-line, comb, max length, password
  shown masked), checkbox, radio group, combo (with optional edit), list box
  (single/multi). Push buttons render but do nothing.
- *Fill mode* starts with the toolbar's **Fill & Sign** button (`pdf.form.edit`).
  Real egui widgets are overlaid on each widget's rectangle at the current scale.
  Tab/Shift+Tab follow the page's tab order (`/Tabs`), falling back to row order.
  Read-only fields are not editable; required fields left empty are listed when
  saving (a warning, not a block).
- Values go through `pdf.form.set {field, value}`, where `field` is the fully
  qualified name. Values are validated against the field type (options must exist,
  max length enforced, UTF-8 ≤ 32 KiB).
- Fields with JavaScript calculate/format/validate actions get a one-line note:
  "Form scripts don't run here, so totals won't update. Use a PDF app for this
  form." (Open buttons were tried and dropped: too much room.) Recognising Acrobat's
  standard calculations (AFSimple_Calculate, simplified field notation) without a
  script engine was considered and deferred (2026-11-20).
- XFA: XFA-only (dynamic) forms show an explanation and no fill mode. Hybrid forms
  are filled through AcroForm and the saved copy drops `/XFA` from `/AcroForm`, so
  XFA-preferring viewers show the filled values. Saving tells the user once.
- If the document's permissions forbid form filling (P bit 9 with bit 6 off) or a
  certification signature forbids changes, fill mode is disabled with the reason.

**D7 Appearance generation.** For each changed field, a new widget appearance
(`/AP /N`) is generated, and for checkboxes/radios `/AS` is switched to the existing
on/off appearance names. Text uses the field's `/DA` (font, size with auto-size for 0,
colour) and `/Q` alignment, with comb and multi-line layout. Glyph widths come from
the font's `/Widths` or the standard 14 metrics. Characters the field's font cannot
encode (WinAnsi for simple fonts; an embedded Type0 font's cmap where present) are
refused at input with an inline message naming the character, instead of producing
a field that shows boxes. `/NeedAppearances` is never set: it would make other
viewers re-render differently, and it is harmful once a document is signed.
*Alternative:* embed and subset a bundled font for any script (`subsetter` plus a
CID font writer). Deferred: it adds a dependency and a font writer for a case most
forms don't hit. The refusal path keeps results honest meanwhile.

**D8 Visual signatures as vector content.**
- *Draw:* a signature pad records strokes (pointer/pen, smoothed, ≤ 64 strokes ×
  2048 points), written as filled vector paths.
- *Type:* the name is laid out in a bundled script-style font (OFL, added to
  `crates/pdf/assets` and the notices) and converted to glyph outlines, so no font is
  embedded in the PDF and any script the font covers works.
- *Image:* PNG/JPEG ≤ 8 MiB and ≤ 4 MP. JPEG passes through as DCTDecode; PNG becomes
  Flate RGB plus an SMask for transparency. An optional "remove white background"
  turns near-white pixels transparent.
- *Placement:* click a signature field (its rectangle is used, aspect kept), or
  drag a rectangle anywhere on a page. Move and resize until saved. Several
  signatures per save are allowed.
- *Writing:* a visual-only signature is written as a Form XObject drawn by an extra
  content stream appended to the page's `/Contents` (as on paper, removable only by
  editing the page). In a signature field, it becomes that field's widget appearance.
  If the document already has digital signatures, a visual-only signature on a page
  is announced first: those signatures will show "changed after signing".
- *Saved signatures:* opt-in "Remember this signature", stored as our own small
  format (paths or the image bytes) in the app data dir under `signatures/`
  (`0700`/`0600`, ≤ 8 entries, ≤ 8 MiB total), deletable from the dialog. Not in
  `prefs.toml`.

**D9 Digital signing: PAdES B-B with RustCrypto.**
- *Certificate:* a `.p12`/`.pfx` chosen with the path picker (any backend, ≤ 1 MiB),
  decrypted with `p12-keystore`. Both modern PBES2/AES and legacy 3DES/RC2 bags are
  read, since many exported certificates are still legacy. The password is a
  `SecretString` passed in the command's params (the same pattern as
  `remote.connect`) and never logged. The decrypted key lives in a `Zeroizing`
  buffer for the duration of one `pdf.sign.digital` call, then is dropped. Nothing
  is cached, so each signature asks again.
- *Keys:* RSA 2048–8192 (PKCS#1 v1.5, `rsa` already in the tree), ECDSA P-256 and
  P-384. Digest SHA-256 (SHA-384 for P-384). The certificate must not be expired at
  signing time, and its key usage must allow digital signatures (a warning, not a
  block, when the extension is absent).
- *Structure:* a signature field (existing empty one, or a new invisible or visible
  one) with a `/Sig` dictionary: `/Filter /Adobe.PPKLite`,
  `/SubFilter /ETSI.CAdES.detached`, `/M`, optional `/Reason`, `/Location` and
  `/ContactInfo` (≤ 256 chars each, user-supplied), and a `/Contents` placeholder of
  a fixed size (16 KiB hex) plus a `/ByteRange` placeholder. The incremental update
  is serialized once, `/ByteRange` is patched with fixed-width numbers, and the
  SHA-2 digest covers the original bytes plus the update except `/Contents`.
- *CMS:* `cms` + `x509-cert` build a detached SignedData with signed attributes
  `contentType`, `messageDigest`, `signingTime` and `signingCertificateV2`
  (ESS, required by PAdES B-B), the signer certificate and any chain certificates
  from the p12. Output larger than the placeholder is an error, not a resize.
- *Order:* form values and visual signatures are written first, in the same
  incremental update; the digital signature is always the last change in a save. A
  second digital signature is a second incremental update on top, so both verify.
- *Alternative considered:* `adbe.pkcs7.detached` as the default subfilter. Kept for
  reading and verification only; CAdES is what current EU and Acrobat validation
  expect.

**D10 Bounds for hostile PDFs.** File ≤ 256 MiB to view (larger shows the metadata
card with the reason); page count reported as is, but only visible pages are parsed;
page box ≤ 14 400 pt (the PDF limit); render output ≤ 16 MP per page/tile and
≤ 192 MiB cache; images inside pages are bounded by hayro's own decoders (JBIG2,
JPX, CCITT) plus our pixel cap; object nesting and xref repair are bounded by the
libraries and our parse budget. hayro cannot be cancelled mid-page, so a page render
that exceeds 10 s shows an error tile and its worker is abandoned and replaced. At
most 2 workers per document can be abandoned before the document stops rendering
and shows "too complex to preview". The lopdf graph for editing is built only for
files ≤ 64 MiB, and fill mode explains the limit above that. Embedded files,
JavaScript, Launch and submit actions, and external references are never acted on.

**D11 Saving.** Commands: `pdf.save` (copy next to the original, name
`<stem> (filled).pdf` or `(signed).pdf`, with ` 2`, ` 3` … on collision),
`pdf.save_as {uri}` (via the path picker, any backend) and `pdf.save_overwrite`
(confirmation dialog first; refused if the file changed on disk since it was loaded,
compared by size + mtime and a hash of the loaded bytes). Every save is
`original bytes + incremental update`, assembled in memory by `fm-pdf::write`, then
written on the target's job lane through the existing temp-file + rename write path,
with conflicts and progress like any copy. After a successful save the Preview moves
to the saved file and the session resets. Encrypted documents: new objects are
encrypted with the document's key, using the same handler (RC4/AES-128/AES-256), so
the saved copy stays protected. If a spike (task 1.3) shows lopdf cannot do this
reliably, fill/sign is disabled for encrypted documents with an explanation, and
viewing is unaffected.

**D12 Unsaved changes.** A dirty `PdfSession` holds the Preview on its document: the
slot stops following the Files cursor and the toolbar shows "Unsaved changes" with
Save, Save As and Discard. Closing the pane, switching workspace layout or quitting
the app goes through the existing unsaved-changes prompt pattern
(Save / Don't save / Cancel). Nothing is autosaved to disk (filled values may be
personal data).

**D13 Verification.** For each `/Sig` field: check `/ByteRange` (four non-negative
numbers, in bounds, covering everything except exactly the `/Contents` hex), parse
the CMS (≤ 64 KiB), recompute the digest, check `messageDigest`, verify the
signature over the signed attributes with the signer certificate's key (RSA
PKCS#1 v1.5/PSS, ECDSA P-256/P-384/P-521; SHA-1/224/256/384/512), and check the
certificate's validity period at the claimed signing time. Results:
- *Valid*: integrity OK and the signed range ends where the file ends.
- *Valid, changed after signing*: integrity OK, and later incremental updates exist.
  Their kind is listed (form fields filled, signatures added, other changes)
  by diffing the revision's objects at a coarse level.
- *Broken*: digest or signature mismatch, malformed structure, unsupported algorithm.
- Weak algorithm (SHA-1, RSA < 2048) is flagged on any result.
- Identity: signer name, issuer, serial, SHA-256 fingerprint, and the line "Signer
  identity not checked against a trust list." There are no network requests.
Verification runs on the PDF render pool after load, at most once per document, with
a 5 s budget. The signatures panel opens from a toolbar badge ("2 signatures").

**D14 Commands (all with id, label, menu path, params doc, `enabled`, fuzz tests).**
- `pdf_view_cmds.rs`: `preview.pdf.page {page}`, `preview.pdf.next`,
  `preview.pdf.prev`, `preview.pdf.first`, `preview.pdf.last`,
  `preview.pdf.fit {mode}`, `preview.pdf.colors {mode}`,
  `preview.pdf.outline.toggle`, `preview.pdf.outline.go {index}`,
  `preview.pdf.link {page, index}`, `preview.pdf.visible {pages}`,
  `preview.pdf.password {password}`, `preview.pdf.signatures.toggle`.
- `pdf_form_cmds.rs`: `pdf.form.edit`, `pdf.form.set {field, value}`,
  `pdf.form.reset`, `pdf.save`, `pdf.save_as {uri}`, `pdf.save_overwrite`,
  `pdf.discard`.
- `pdf_sign_cmds.rs`: `pdf.sign.visual {kind, data}`,
  `pdf.sign.place {page, rect, field?}`, `pdf.sign.move {id, rect}`,
  `pdf.sign.remove {id}`, `pdf.sign.digital {cert, password, field?, page?, rect?,
  reason?, location?, contact?}`, `pdf.sign.saved.{store,delete}`.
Text Find and copy reuse `preview.find.*` and `preview.text.*`.

**D15 UI (`fm-ui`).** `preview_pdf.rs` (pages, tiles, selection, Find highlights,
link hover), `preview_pdf_form.rs` (field overlay), `pdf_sign.rs` (signature dialog:
Draw/Type/Image tabs, saved signatures, digital tab with certificate, password,
reason/location), `pdf_signatures.rs` (verification panel). Colours only from
`Tokens` (new tokens: `pdf_page_border`, `pdf_gap`, `field_fill`,
`field_focus`, `sig_valid`, `sig_changed`, `sig_broken`), in both themes. Snapshot
tests use a small generated PDF fixture so they don't depend on system fonts.

## Risks / Trade-offs

- [Two parsers can disagree on a damaged file: hayro repairs it differently than
  lopdf] → fill/sign uses lopdf's view of objects but our appearances are re-rendered
  by hayro before saving. A document lopdf can't open gets view-only with an
  explanation. Round-trip tests run on a fixture corpus (simple, hybrid XFA,
  encrypted, already signed, incremental, xref streams).
- [hayro timeouts/OOM on hostile files, as its README admits] → D10 caps, render
  deadline with worker replacement, catch_unwind, fuzz-style corpus tests.
- [lopdf incremental writer output rejected by strict readers] → validate saved
  files in tests by reopening with hayro and lopdf, plus `qpdf --check` in CI when
  available (skipped otherwise, like the Docker SMB tests).
- [A signature that verifies for us but not in Acrobat] → follow PAdES B-B exactly
  (signingCertificateV2, CAdES subfilter, ByteRange layout). Verify our output with
  our own verifier and, in an optional CI step, with `pdfsig` (poppler) when present.
  Manual Acrobat Reader check is in the smoke list.
- [`rsa` crate timing side channel (RUSTSEC-2023-0071, Marvin)] → it concerns
  decryption oracles observable over a network. Here signing runs locally, once per
  user action, with no oracle. Recorded in the security spec; revisited when
  `rsa` ships a constant-time fix.
- [Dependency weight] → about 10 new direct crates, mostly RustCrypto pieces that
  share foundations (`der`, `spki`, `digest`) with crates already in the tree via
  SSH. hayro's embedded CMaps add about 250 KB to the binary. Measured in task 1.2
  (binary size and clean build time before and after).
- [Filled values outside the font's encoding] → refused at input (D7) rather than
  saved wrong. Font embedding is a follow-up.
- [Users expect saved signatures to sync between machines] → stated as local-only
  in the dialog.

## Migration Plan

Additive. PDFs that showed a metadata card now show pages; the metadata stays
reachable as an info section. Prefs gain a `pdf` section (`colors`, `fit`) with
`serde(default)`. Removing the feature means deleting the `signatures/` folder. No
data format changes elsewhere.

## Open Questions

All settled during implementation:

- *Encrypted documents (D11, task 1.3):* lopdf 0.47 encrypts the objects of an
  incremental update with the document's key and handler. Round trips on RC4,
  AES-128 and AES-256 fixtures reopen in hayro and lopdf with the filled values, so
  filling and visual signatures work on encrypted documents. Digital signing of an
  encrypted document is refused with an explanation (the signed byte ranges would
  have to cover encrypted `/Contents` handling we do not test); viewing is
  unaffected.
- *Signature font (task 6.2):* Dancing Script (OFL-1.1, Latin and Latin Extended),
  bundled in `crates/pdf/assets`. Typed names are converted to outlines with
  `ttf-parser`; the font is never embedded in saved PDFs. Its licence is in About.

## Implementation notes

- *PKCS#12:* instead of `p12-keystore`, `fm-pdf::cert` reads the container with the
  `pkcs12`/`pkcs5`/`pkcs8` RustCrypto types: PBES2 (PBKDF2 + AES-CBC) and the legacy
  `pbeWithSHAAnd3-KeyTripleDES-CBC` / `pbeWithSHAAnd40BitRC2-CBC` bags, MAC verified
  with HMAC-SHA1/SHA-256. This keeps every crypto dependency on one RustCrypto
  release line and avoids a second ASN.1 stack.
- *Placeholder size:* `/Contents` reserves 16 KiB of signature bytes (32 KiB of hex),
  enough for an RSA-8192 signature with a chain of several certificates; output that
  does not fit is an error, not a resize.
- *Saving:* `OpRequest::WriteBytes` writes through `fm_ops::write_bytes` (temp file +
  rename, conflict prompt for Save As, first free ` 2`, ` 3` name for copies). The
  overwrite check compares the target with the loaded bytes (size first, then the
  content) right before writing, which subsumes the size + mtime + hash comparison.
- *Remembered signatures:* drawn and typed signatures are stored as small JSON files,
  images as their original bytes, one file per signature in `<data>/signatures`.
