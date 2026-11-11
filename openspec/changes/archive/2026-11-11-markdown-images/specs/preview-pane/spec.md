## MODIFIED Requirements

### Requirement: Native document-quality Markdown
Markdown SHALL be rendered natively, without a webview or editor, using a bounded
CommonMark document tree with tables, task lists and strikethrough enabled.
Paragraphs SHALL use a proportional reading face with real regular/bold weights,
italic, combined emphasis, inline-code styling, link styling and preserved hard
line breaks. Headings SHALL have distinct scale/weight and document spacing;
soft breaks SHALL flow as spaces. Document width SHALL be limited for readability.

Lists SHALL preserve nesting, ordered start numbers, multiple paragraphs and
embedded blocks. Task checkboxes SHALL show their checked state without permitting
edits. Blockquotes SHALL retain nested paragraphs/blocks with an accent edge.
Fenced code SHALL show its language token and semantic syntax highlighting
computed on the preview worker, with a plain-text fallback for unknown languages
and overlong lines. Code SHALL preserve whitespace and support wrapping or
horizontal scrolling.

Tables SHALL retain header cells, rows, inline formatting and Markdown column
alignment. Each row SHALL have one shared height derived from its tallest cell,
continuous column boundaries and top-aligned cell text. Text SHALL NOT be
justified or letter-spaced to fill cells. Narrow panes SHALL scroll tables
horizontally rather than squeeze columns below a readable minimum.

HTML SHALL remain literal inert text. Images SHALL be rendered as specified in
"Markdown images"; links SHALL NOT be fetched or opened except by an explicit click.
Markdown SHALL enforce a 1 MiB source cap, the configured text-line cap,
5 000 block/list-item nodes, nesting depth below 32, 30 000 inline spans,
4 096 table cells and 64 columns. Exceeding any cap SHALL show truncation.

#### Scenario: Preserve document structure
- **WHEN** Markdown includes a loose nested ordered list, checked/unchecked tasks,
  nested blockquotes, a table and fenced Rust code
- **THEN** all structures remain distinct, ordered numbers are preserved, tasks are
  read-only and the code is highlighted without UI-thread parsing
- **AND** Ctrl+A/Ctrl+C includes all loaded blocks, task markers and tab-separated
  table cells in document order without formatting syntax

#### Scenario: Uneven table cells
- **WHEN** one table column has short symptoms and another has multi-line explanations with inline code
- **THEN** both cells start at the same row top, the next row starts below the
  taller cell, borders remain connected, and ordinary word spacing is preserved
- **AND** resizing or zooming does not cause staggered rows or overlapping columns

## ADDED Requirements

### Requirement: Markdown images
Rendered Markdown SHALL show images written as `![alt](url "title")`. Images in paragraphs
SHALL be placed as block-level rows directly after the paragraph text, consecutive images
forming one wrapping row; an image inside a link SHALL keep that link. Images in headings
and table cells SHALL remain `[Image: alt]` placeholders. At most 200 images per document
SHALL be rendered; further ones stay placeholders.

A relative URL SHALL be resolved against the previewed file with the same rules as
Markdown links (percent-decoded, `?query` and `#fragment` ignored, `.` and `..` resolved,
refused above the root) and read through the file's own backend. Absolute paths, `file:`,
`data:`, `//` URLs and other schemes SHALL NOT be loaded. Supported formats are those
of image previews. SVG images are rasterized without text or embedded/external resources.

`http` and `https` images SHALL be loaded only when Preferences > Preview > "Load remote
images in Markdown" is enabled; it SHALL default to off. While off, remote images SHALL
show their alt text with a "Remote image blocked" hover hint and SHALL cause no network
access.

Images SHALL load off the UI thread; until ready, and when blocked, missing, oversized,
undecodable or failed, the alt-text placeholder SHALL be shown and the document SHALL
remain usable. Images SHALL scale down to the document width, never up past natural size,
follow preview magnification, show alt text/title on hover, and render correctly in
dark and light themes. Copy SHALL treat an image as its alt text; Find does not search alt text.

#### Scenario: Local relative image
- **WHEN** `README.md` contains `![logo](img/logo.png)` and `img/logo.png` exists
- **THEN** the rendered view shows the image below its paragraph without blocking a frame

#### Scenario: Missing or escaping path
- **WHEN** an image points at a missing file or at `../../../etc/x.png`
- **THEN** the alt-text placeholder is shown, nothing outside the root is read, and the preview stays usable

#### Scenario: Remote images off by default
- **WHEN** Markdown contains `![badge](https://example.com/b.svg)` and the preference is off
- **THEN** no connection is made and the placeholder shows "Remote image blocked" on hover

#### Scenario: Remote images enabled
- **WHEN** the preference is enabled and the URL serves a valid image within limits
- **THEN** the image appears; toggling the preference re-renders open Markdown previews

#### Scenario: Linked badge
- **WHEN** an image is wrapped in a link and clicked
- **THEN** the link is followed by the normal Markdown link rules
