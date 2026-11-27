## Why

Rich Text Format files are everywhere in exported mail, bank and insurance letters, licence texts and old WordPad notes. Today Preview shows them as a hex dump or as raw `{\rtf1…}` codes, so the user has to open Word or LibreOffice just to read one. The Markdown preview already renders paragraphs, emphasis, lists, tables, links and images natively, so RTF can be read in the pane with a reader that turns RTF into that same document model.

## What Changes

- New: RTF files (`{\rtf` content, `.rtf` names) preview as a formatted document. Supported: paragraphs, bold/italic/strikethrough, headings taken from the style sheet, bulleted and numbered lists, tables, hyperlinks, `\u` Unicode and `\'xx` escapes decoded per font code page (cp1250/1251/1252/1253/1254/1255/1256/1257/874 and Symbol), and embedded pictures: PNG and JPEG directly, and Windows metafiles (WMF/EMF) through the bitmap they hold.
- New: a **Source** toggle shows the RTF text, like Markdown's; Find, Select all and Copy work on the rendered text.
- New: RTF is read with its own size cap (larger than Markdown's 1 MiB, because pictures are hex-encoded inside the text). Pictures are decoded lazily on the preview image pool with pixel and byte caps; a picture that cannot be shown (pure vector metafile, unknown format, over the cap) shows a labelled placeholder.
- New: Preferences > Preview gets an **RTF** entry in the filename filter list (default `*.rtf`).
- Unchanged: double-click and Enter keep opening an RTF in its default application (Word, LibreOffice, WordPad); the built-in editor does not edit RTF. A file named `.rtf` that is not RTF (no `{\rtf` header) falls back to the normal text preview.
- Security: an RTF is inert. Embedded OLE objects (`\object`), linked or remote pictures (`INCLUDEPICTURE`, `\*\picprop` links), field codes other than HYPERLINK, and macros are never run, loaded or fetched; links open only on an explicit click.

## Capabilities

### New Capabilities
- `rtf-preview`: reading RTF into the native document view: supported constructs, code pages, embedded pictures, the Source view, limits and graceful degradation.

### Modified Capabilities
- `preview-pane`: "Supported previews" lists RTF; "Configurable preview filename filters" gains the RTF type; "Content is identified by sniffing" recognises the `{\rtf` header.
- `security`: RTF documents are inert (no objects, no external resources, bounded parsing).

## Impact

- `crates/preview`: new `rtf` module (tokenizer, destination handling, code pages, RTF → `MdDoc` blocks, picture extraction), `Kind::Rtf` in `sniff`, `Preview::Markdown` reused for rendering (or a thin `Preview::Rtf` wrapper if the Source view needs it). Filename filter defaults.
- `crates/engine`: embedded pictures served to the existing Markdown image pool by a document-local key instead of a URL; preview limits for RTF.
- `crates/ui`: Source toggle and the placeholder for pictures that cannot be shown; Preferences filter list.
- Dependencies: none new (`encoding_rs` and `image` are already in the build). No licence changes.
- Tests: fixtures derived from `tests/test-rtf/` (shrunk where large), hostile inputs (deep nesting, endless groups, huge `\bin`, broken hex, lying picture sizes), snapshots in dark and light.
