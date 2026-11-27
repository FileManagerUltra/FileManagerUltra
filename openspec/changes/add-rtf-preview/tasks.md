## 1. Fixtures and groundwork

- [ ] 1.1 Fixtures in `crates/preview/tests/fixtures/rtf/` derived from `tests/test-rtf/`: the Word letter (tables, lists, links, code pages), `lic.rtf` (cp1250, headings), one `\wmetafile8` picture cut from `rozenbottel.rtf` (shrunk), the non-RTF `Readme.rtf`; plus synthetic files for cp1251/1253, a double-byte run, `\pngblip`/`\jpegblip`/`\emfblip`/`\dibitmap`, a vector-only WMF, `\trhdr`, `INCLUDEPICTURE` and `\object`
- [ ] 1.2 `Kind::Rtf` in `fm_preview::sniff` (`{\rtf` after an optional BOM); a `.rtf` name without the header keeps content detection; tests
- [ ] 1.3 RTF filename filter group (`*.rtf`, below Markdown) with migration of stored settings (as the SVG group did); Preferences > Preview lists it; tests

## 2. Reader (`fm_preview::rtf`)

- [ ] 2.1 Tokenizer: groups, control words with parameters, control symbols, `\'hh`, `\binN`, text; caps on word length, parameter range, nesting depth (64) and source size (64 MiB); fuzz test that random and truncated input never panics
- [ ] 2.2 Destinations: skip font/colour/style/info/header/footer/footnote/annotation groups, hidden text and unknown `\*` destinations; read the font table (`\fcharset` per font) and style sheet (heading names, `\outlinelevel`)
- [ ] 2.3 Code pages: `\fcharset` and `\ansicpg` to `encoding_rs` (incl. double-byte runs), Symbol mapping, `\uN` with `\ucN` skipping, U+FFFD for the undecodable; tests per code page
- [ ] 2.4 Blocks: paragraphs and `\line`, bold/italic/strike runs, headings, lists from `\pntext`/`\listtext`, tables (`\trowd`/`\cell`/`\row`, `\trhdr` header), `HYPERLINK` fields as links, other fields as their result text; Markdown block/inline/cell caps and the truncated flag
- [ ] 2.5 Pictures: record kind, goal size and file byte range of each `\pict` (max 200) in `MdDoc.embedded`, referenced as `embedded:N` from `MdImage`; `\object` shows only its `\result`
- [ ] 2.6 Source text: RTF with picture data collapsed to `[picture: N KiB]` markers, capped like text

## 3. Engine

- [ ] 3.1 Preview worker reads RTF through the backend with the RTF cap and builds the document off the UI thread (local, SMB, SFTP, archive members)
- [ ] 3.2 `md_images`: `embedded:N` resolves to a range read of the previewed file, bounded hex/`\bin` decode, then PNG/JPEG as is, WMF (`META_STRETCHDIB`/`META_DIBSTRETCHBLT`) and EMF (`EMR_STRETCHDIBITS`/`EMR_BITBLT`) bitmap extraction and DIB → BMP; reasons for placeholders (vector only, unknown, damaged, over limit); `catch_unwind`; tests including lying sizes
- [ ] 3.3 Graceful-failure tests: hostile nesting, unclosed groups, broken hex, `\bin` past the end, 100 000 × 100 000 picture header

## 4. UI

- [ ] 4.1 RTF previews render through the Markdown view; Source toggle; Find, selection, copy and zoom work; placeholder card for pictures that cannot be shown
- [ ] 4.2 Snapshots in dark and light: the Word letter (table, list, link), `lic.rtf` (cp1250 heading), the picture fixture, a placeholder, the Source view

## 5. Security, docs and wrap-up

- [ ] 5.1 Tests for the `security` requirement: `INCLUDEPICTURE`/`INCLUDETEXT`/`DDE` cause no file or network access, `\object` is not decoded, links only on click
- [ ] 5.2 `docs/architecture.md` (preview kinds, embedded image source), user-facing CHANGELOG line
- [ ] 5.3 Manual check on Linux and Windows with every file in `tests/test-rtf/` (text, code pages, pictures in the 37 MiB file, Source, opening in the default app)
