## ADDED Requirements

### Requirement: RTF documents preview as formatted text
Preview SHALL show a Rich Text Format file (content starting with `{\rtf`, optionally after a UTF-8 BOM, or matching the RTF filename filter) as a native formatted document, rendered with the same document view, zoom, themes, Find, selection and copy as Markdown. It SHALL support paragraphs and line breaks; bold, italic and strikethrough runs; headings taken from style-sheet names `heading 1` to `heading 6` or `\outlinelevel`; bulleted and numbered lists from `\pntext`/`\listtext`; tables from `\trowd … \cell … \row` (with a header row when `\trhdr` is present, headerless otherwise); and hyperlinks from `HYPERLINK` fields, opened only by an explicit click. Font tables, colour tables, style sheets, document info, headers, footers, footnotes, annotations, hidden text (`\v`) and destinations marked `\*` that are not understood SHALL be skipped without showing their contents.

#### Scenario: A Word letter with a table and links
- **WHEN** an RTF written by Word with paragraphs, a table and a hyperlink is previewed
- **THEN** the paragraphs read in order with their bold and italic text, the table shows its rows and cells, and the link text is clickable and opens only when clicked

#### Scenario: Headings and lists
- **WHEN** an RTF uses the style sheet's `heading 1` style and a bulleted list
- **THEN** the heading is shown as a heading and the list items as a list

#### Scenario: Skipped groups stay hidden
- **WHEN** an RTF has a font table, document information, a header and hidden text
- **THEN** none of their text appears in the preview

### Requirement: RTF text is decoded per code page
Characters SHALL be decoded from `\'hh` escapes with the code page of the current font (`\fcharset`: 0, 161, 162, 163, 177, 178, 186, 204, 222, 238 and the double-byte 128, 129, 134, 136), falling back to `\ansicpg`, and from `\uN` (a signed 16-bit value, negative values offset by 65 536) followed by skipping the next `\ucN` characters (default 1). Text in the Symbol font (`\fcharset2`) SHALL map to Unicode where a mapping exists. Characters that cannot be decoded SHALL become U+FFFD, never an error.

#### Scenario: Central European licence text
- **WHEN** an RTF with a `\fcharset238` font contains `\'af` and `\'d1`
- **THEN** they are shown as `Ż` and `Ń`

#### Scenario: Unicode with a fallback character
- **WHEN** the text contains `\uc1荤?`
- **THEN** a single `€` is shown and the `?` fallback is not

### Requirement: Embedded pictures are shown
Pictures in `\pict` groups SHALL be shown in place, scaled to their goal size (`\picwgoal`/`\pichgoal`) within the pane width: PNG (`\pngblip`) and JPEG (`\jpegblip`) as stored, device-independent bitmaps (`\dibitmap`) and Windows metafiles (`\wmetafile`, `\emfblip`) through the first bitmap they contain. Pictures SHALL be decoded only when they come into view, on the preview image pool, with the image byte, pixel, count and cache limits of Markdown images. A picture that cannot be shown (a metafile with only vector drawing, an unknown format, damaged data or a picture over a limit) SHALL show a placeholder naming the reason, and the rest of the document SHALL still be shown.

#### Scenario: Pictures in a WordPad file
- **WHEN** an RTF holds `\wmetafile8` pictures that wrap a bitmap
- **THEN** each picture appears at its place in the text once it scrolls into view

#### Scenario: A vector-only metafile
- **WHEN** a picture is a metafile without a bitmap
- **THEN** a placeholder "Picture (vector drawing) not shown" appears and the text around it is shown normally

#### Scenario: A large file opens quickly
- **WHEN** a 37 MiB RTF made mostly of pictures is previewed
- **THEN** its text appears without first decoding every picture, and pictures load as they come into view

### Requirement: RTF has a source view
The preview toolbar SHALL offer a Source toggle for RTF, like Markdown's, showing the RTF text with each picture's data replaced by a short `[picture: N KiB]` marker. Opening an RTF with Enter or a double-click SHALL keep handing it to its default application; the built-in editor SHALL NOT edit RTF.

#### Scenario: Toggle to source
- **WHEN** the user presses Source on an RTF preview
- **THEN** the RTF control words are shown as text, picture data is collapsed to markers, and pressing it again returns to the formatted view

### Requirement: RTF reading is bounded and degrades gracefully
Reading SHALL stop at 64 MiB of source, 64 levels of group nesting, the Markdown limits for blocks, inline runs and table cells, and 200 pictures; control words longer than 32 characters and numeric parameters outside the 32-bit range SHALL be treated as text or ignored. Reaching a limit SHALL show the document as truncated, not fail. A file named like RTF without an `{\rtf` header SHALL be previewed by content detection (as text when it is text). Unbalanced braces, a missing closing brace, broken hex and a `\bin` length past the end of the file SHALL never panic.

#### Scenario: A text file named .rtf
- **WHEN** `Readme.rtf` holds plain cp1252 text without an `{\rtf` header
- **THEN** it is previewed as text

#### Scenario: Hostile nesting
- **WHEN** an RTF nests 100 000 groups or never closes its first brace
- **THEN** the preview shows what was read up to the limit, marked truncated, without hanging or crashing
