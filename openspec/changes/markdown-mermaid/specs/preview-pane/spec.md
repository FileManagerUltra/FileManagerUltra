## ADDED Requirements

### Requirement: Mermaid diagrams in Markdown
A rendered Markdown preview SHALL draw a fenced code block whose language (the first word of the info string, case-insensitive) is `mermaid` as a diagram when Preferences > Preview > "Draw Mermaid diagrams" is on; the preference SHALL default to on, and when it is off such blocks SHALL be ordinary code blocks. Supported diagram types SHALL be flowchart (`flowchart`, `graph`), sequence, class, state, entity-relationship, Gantt, pie, user journey, timeline, git graph and mind map, with subgraphs, node and edge labels (including `<br/>` line breaks), and solid, dotted, thick, undirected and two-way edges.

Diagrams SHALL use the app's current theme (dark or light, contrast and accent settings) for background, shapes, borders, text, lines, subgroup boxes and labels; theme settings in the document's own `%%{init}%%` directive SHALL NOT override them. A diagram SHALL scale down to the document width, never up past its natural size, and follow preview magnification, like Markdown images. While a diagram is being drawn, a placeholder SHALL reserve its space; the rest of the document SHALL stay usable.

A diagram SHALL keep the code block's header (language label and copy control, which copies the Mermaid source exactly) and add two icon controls, each with a tooltip and accessible name: "Copy diagram as image", which puts the drawn diagram on the clipboard as an opaque picture on its background colour with a margin, and "Show source", which shows that one block as its highlighted source code, with a "Show diagram" icon control to draw it again; the choice SHALL apply to the current document only, and a block shown as source SHALL be searched by Find like code. Whole-document copy SHALL contain the source as code text. The diagram SHALL be exposed to assistive technology as "Mermaid diagram" with its type, described by its source (first 1000 characters). The Markdown source view SHALL show the Mermaid source.

When a diagram cannot be drawn (unknown or unsupported type, syntax error, a limit exceeded, a render failure or timeout), the block SHALL be shown as an ordinary highlighted code block with one line above it: "Diagram not drawn: " followed by a short reason. At most 50 diagrams SHALL be drawn per document; later ones SHALL be code blocks with the reason "Too many diagrams in this document".

#### Scenario: Flowchart with subgroups
- **WHEN** a Markdown file contains a `mermaid` block with `flowchart TB`, four `subgraph` blocks, chained edges, labelled edges and a dotted edge
- **THEN** the rendered view shows one diagram with every node, subgroup title and edge label readable, in the current theme's colours, without blocking a frame

#### Scenario: Theme and zoom
- **WHEN** the user switches between dark and light, or zooms the Preview in
- **THEN** the diagram is redrawn in the new colours or at the new size and its labels stay inside their boxes

#### Scenario: Broken diagram
- **WHEN** a `mermaid` block contains `flowchart LR` followed by `A[[[ --> }}}`
- **THEN** the block is shown as code with "Diagram not drawn: " and the parse problem above it, and the rest of the document renders normally

#### Scenario: Copy the source
- **WHEN** the user activates the copy control on a drawn diagram
- **THEN** the clipboard contains exactly the Mermaid source of that block

#### Scenario: Show one diagram's source
- **WHEN** the user activates "Show source" on a drawn diagram
- **THEN** that block alone is shown as its highlighted Mermaid source with a "Show diagram" button, other diagrams stay drawn, and opening another file shows all of its diagrams drawn

#### Scenario: Copy a diagram as an image
- **WHEN** the user activates "Copy diagram as image" on a drawn diagram
- **THEN** the clipboard holds the diagram as shown, without transparent pixels, so it stays readable when pasted on any background

#### Scenario: Clear the diagram cache
- **WHEN** the user activates Preferences > Advanced > "Clear diagram cache" (or Preview > Clear Diagram Cache)
- **THEN** every drawn diagram and remembered failure is forgotten, queued and running drawing work is dropped, and open documents draw their diagrams again

#### Scenario: Preference off
- **WHEN** "Draw Mermaid diagrams" is turned off
- **THEN** every `mermaid` block is shown as an ordinary code block on the next frame, and Find searches its text

## MODIFIED Requirements

### Requirement: Preview has pane-local Find
`Ctrl+F`, the Preview menu and a toolbar Find button (non-log views) SHALL open/focus Find in the
focused Preview, without opening the Files-pane filter. Text, structured text
(including raw/pretty mode) and rendered Markdown SHALL offer a query field,
case-sensitive, whole-word and regular-expression options, a current/total match
counter, Previous/Next buttons, and distinct current/other match highlighting in
both themes. Searches SHALL use already-loaded content only, not read files or
search hidden Markdown syntax. Text is searched per source line; Markdown is
searched per rendered paragraph/heading, code block or table cell; a Mermaid
code block drawn as a diagram is not searched (it has no text to highlight) and
is searched as code whenever it is shown as code.

#### Scenario: Navigate matches without leaving Find
- **WHEN** a query has matches and the user presses Enter/F3 or Shift+Enter/Shift+F3
- **THEN** the next/previous match is revealed vertically and horizontally, wrapping at either end
- **AND** Enter/Shift+Enter work while the query retains keyboard focus
- **AND** Escape/Close hides Find and its highlights without changing the document or its copyable text
- **AND** line-number gutters, code-language labels, copy buttons and other UI chrome are not searched

#### Scenario: Bounded asynchronous matching and content changes
- **WHEN** a query, option, loaded document or raw/pretty mode changes
- **THEN** matching runs off the UI thread, at most one worker per pane, and results for obsolete inputs are discarded
- **AND** the current match resets and match locations are recomputed for the new displayed content
- **AND** text rendering remains virtualized when wrapping is off
- **AND** queries are limited to 1024 Unicode characters, regex compilation is bounded, and at most 10 000 matches are retained with a visible `+` when more exist
- **AND** invalid patterns and worker failures are shown inline; empty/no-match queries disable navigation

#### Scenario: Content-specific search and keyboard ownership
- **WHEN** Ctrl+F is used in a log preview
- **THEN** the existing record-filter panel opens and its search field gains focus, retaining severity/time/regex controls and applied filters
- **AND** logs expose only the Filter toolbar button, not a duplicate Find button
- **AND** Escape hides that panel without clearing its filters
- **WHEN** the Preview is an image, video, folder, binary, error or multi-file card grid
- **THEN** Find is disabled with an explanation rather than implying unavailable text/OCR/byte search
- **AND** Files Ctrl+F, terminal input, dialogs and query-field Ctrl+A/Ctrl+C retain their existing behavior

The `preview.find.{open,close,set,next,previous}` commands SHALL validate their
parameters and remain pane-local; `set` accepts query/case/word/regex options and
the other commands accept only an optional pane.

The Find control SHALL share the main source/title toolbar row (preview zoom is in the pane header, see `panes` "Pane zoom"),
not consume a separate row. At narrow widths view controls SHALL move into the
existing View options overflow menu; the filename SHALL truncate to keep that
menu and the log Filter button visible. An open search/filter panel MAY appear
below the toolbar.
