## Context

Preview renders Markdown natively from a bounded document model (`fm_preview::markdown::MdDoc`: headings, paragraphs of `Inline` runs with bold/italic/strike/code/link, lists, quotes, tables with per-cell images, image rows, rules). Images in that model are only described (`MdImage.url`); the engine's `md_images` service loads, decodes and caches them on its own bounded pools, and the UI asks it what it may draw.

RTF files today fall through to text or hex. The sample set in `tests/test-rtf/` shows what real files need:

| Sample | What it uses |
|---|---|
| `Afspraakbevestiging…RTF` (Word, 96 KiB) | 11 font code pages, 22 table rows, list markers, hyperlinks, `\u` and `\'xx` |
| `lic.rtf` (WordPad, 44 KiB) | cp1250 (`\fcharset238`) with 1 500 `\'xx` escapes, style-sheet headings, links |
| `rozenbottel.rtf` (2 MiB), `driehoekgrannies-vlaggetjes.rtf` (37 MiB) | `\pict\wmetafile8` pictures: a WMF whose only drawing is a StretchDIBits record holding a 24-bit DIB |
| `Readme.rtf` | not RTF at all: cp1252 plain text named `.rtf` |

Constraints: pure Rust, no `unsafe`, no new dependencies or licences; hostile input is bounded (golden rules 1, 8); the UI thread never blocks; works the same on local, SMB, SFTP and inside archives.

## Goals / Non-Goals

**Goals:**
- Read RTF into the existing `MdDoc` so the Markdown renderer, Find, selection, copy, zoom and themes work unchanged.
- Decode text correctly per font code page and `\uc` skip counts.
- Show embedded PNG, JPEG and bitmap-bearing WMF/EMF pictures, decoded lazily and bounded.
- Degrade visibly: anything not understood becomes plain text or a labelled placeholder, never a crash or a hex dump.

**Non-Goals:**
- Editing RTF, or writing RTF.
- Exact page layout: fonts, sizes, colours, indents, tab stops, columns, headers/footers, page breaks, text boxes and drawing shapes are not reproduced (colour and font size may come later).
- Rendering vector WMF/EMF drawing records (only the embedded bitmap is used).
- OLE objects, embedded fonts, revision marks, comments (annotations are skipped).
- Thumbnails of RTF files.

## Decisions

**D1. Own tokenizer and reader instead of a crate.** RTF syntax is small (groups, control words with optional numeric parameter, control symbols, `\'hh`, `\binN`, text). The available crates (`rtf-parser` and others) are immature, do not handle code pages per font, pictures or tables, and would still need hardening. A hand-written reader of a few hundred lines in `fm_preview::rtf` is easier to bound and fuzz. *Alternative:* `rtf-parser` 0.4: rejected for missing charset and picture support and unclear behaviour on hostile input.

**D2. Map to `MdDoc`, reuse the Markdown renderer.** The reader keeps a group stack of character state (bold, italic, strike, font → code page, `\uc` count, hidden) and paragraph state (style → heading level, list marker, in-table). `\par` ends a paragraph; style-sheet names `heading 1…6` (localised names fall back to `\outlinelevelN`) give `MdBlock::Heading`; `\pntext`/`\listtext` with `\ls`/`\ilvl` give list items; `\trowd … \cell … \row` give a headerless `Table`; `{\field{\*\fldinst HYPERLINK "url"}{\fldrslt text}}` gives `Inline.link`. Unknown destinations marked `\*` and the font, colour, style, info, header/footer and annotation groups are skipped. *Alternative:* a dedicated `Preview::Rtf` renderer: rejected, it would duplicate selection, Find and zoom.

**D3. A thin wrapper for the Source view.** `Preview::Markdown` already carries `source: TextDoc` for its Source toggle, so RTF reuses it: the RTF text, with picture hex runs collapsed to a `[picture: N KiB hex]` marker so the source stays readable and small. The Source view is capped like other text.

**D4. Code pages.** `\fcharsetN` per font maps to `encoding_rs`: 0→windows-1252, 161→1253, 162→1254, 163→1258, 177→1255, 178→1256, 186→1257, 204→1251, 222→874 (windows-874), 238→1250; 128/129/134/136 (Shift-JIS, EUC-KR, GBK, Big5) are double-byte, so consecutive `\'hh` bytes of one run are decoded together; 2 (Symbol) maps the Symbol private range to Unicode where a mapping exists and to `?` otherwise. `\ansicpgN` is the default. `\uN` (signed 16-bit) followed by skipping the next `\ucN` characters wins over the fallback.

**D5. Pictures stay in the file; the image pool reads them on demand.** While reading, a `\pict` group records only its kind (`\pngblip`, `\jpegblip`, `\wmetafile`, `\emfblip`, `\dibitmap`), goal size (`\picwgoal`/`\pichgoal`, twips → pixels) and the byte range of its hex (or `\bin`) data in the file. The document holds these in `MdDoc.embedded` and refers to them from `MdImage.url` as `embedded:N`. `md_images` treats `embedded:` as a document-local source: it reads that byte range through the document's own backend, hex-decodes it (bounded by `MAX_BYTES`), and extracts:
- PNG/JPEG: as is;
- WMF: walk records, take the first `META_STRETCHDIB` (0x0F43) or `META_DIBSTRETCHBLT` (0x0B41) bitmap, prepend a `BITMAPFILEHEADER`, decode as BMP;
- EMF: same with `EMR_STRETCHDIBITS` (81) / `EMR_BITBLT` (76);
- DIB: prepend the file header.
A metafile without a bitmap gives "Picture (vector drawing) not shown". This keeps a 37 MiB file's pictures out of memory until they scroll into view, uses the existing pixel, byte, count and cache caps, and works on remote backends. *Alternative:* decode all pictures while reading: rejected for memory and time.

**D6. Limits.** RTF reading has its own source cap (64 MiB) because hex pictures dominate size; text, block, inline and cell caps are the Markdown ones (`MAX_BLOCKS`, `MAX_INLINES`, `MAX_CELLS`), group depth is capped (64), a single control word or parameter is capped (32 chars / ±2³¹), `\binN` is capped by the source cap, and at most 200 pictures are kept. Hitting a cap marks the document truncated (shown like Markdown's).

**D7. Sniffing and filters.** `sniff` recognises `{\rtf` (after an optional UTF-8 BOM) as `Kind::Rtf`; the RTF filename group (`*.rtf`) is added to the filter list above Text. A `.rtf` file without the header goes to content detection and so to text, which is how `Readme.rtf` is shown. Double-click keeps opening the default application (already the case: `rtf` is a document extension for the editor).

## Risks / Trade-offs

- [Layout fidelity: letters look different from Word] → The preview is for reading; Source and Open in default app are one click away. Font size and colour can be added later without changing the model much.
- [Hostile files: deep nesting, endless groups, giant `\bin`, broken hex] → Every cap in D6, fuzz tests through the reader, decoding only on the image pool with the existing caps and `catch_unwind`.
- [Code-page mistakes give mojibake] → Fixtures from the samples per code page (cp1250 Polish, cp1252 Dutch, plus synthetic cp1251/cp1253 and a double-byte run).
- [WMF/EMF variants without a bitmap] → Placeholder text; vector rendering is out of scope.
- [Large files read twice (text pass, then picture ranges)] → Pictures are read only when visible, by range; the text pass skips hex quickly.

## Migration Plan

New preview kind and a new filter group; stored filter settings without an RTF group gain one above Text on load (like the SVG migration). No other data changes. Rollback: removing the module leaves `.rtf` files with the text preview again.

## Open Questions

- Should table header rows (`\trhdr`) become a table header? (Planned: yes when present, otherwise headerless.)
- Text colour and highlight: worth a follow-up once the base preview has been used for a while.
