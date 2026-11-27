## ADDED Requirements

### Requirement: RTF documents are inert and bounded
An RTF document SHALL be treated as untrusted data. Embedded OLE objects (`\object`, `\objdata`), embedded fonts, macros and field codes other than `HYPERLINK` (such as `INCLUDEPICTURE`, `INCLUDETEXT`, `DDE`, `DDEAUTO`) SHALL never be run, loaded, opened or fetched; at most their result text (`\fldrslt`) is shown. Linked pictures and any URL or path in the document SHALL NOT cause a file or network access; a hyperlink opens only on an explicit click and only through the same link handling as Markdown. Reading SHALL use the limits of `rtf-preview` and SHALL run on the preview worker, picture decoding on the bounded image pool inside `catch_unwind`, so a hostile file can neither block the interface nor exhaust memory.

#### Scenario: A field that would load a remote picture
- **WHEN** an RTF contains `{\field{\*\fldinst INCLUDEPICTURE "http://example.test/x.png" \d}{\fldrslt picture}}`
- **THEN** no network or file access happens and only the result text is shown

#### Scenario: An embedded OLE object
- **WHEN** an RTF holds an `\object` group with `\objdata`
- **THEN** the object is neither decoded nor started; its `\result` picture or text is shown when present, otherwise nothing

#### Scenario: Oversized picture data
- **WHEN** a `\pict` claims a 100 000 × 100 000 bitmap or holds more than the image byte limit
- **THEN** a placeholder is shown, the rest of the document is unaffected, and memory use stays within the image cache limit
