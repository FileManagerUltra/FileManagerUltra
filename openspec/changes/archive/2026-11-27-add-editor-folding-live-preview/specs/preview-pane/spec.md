## ADDED Requirements

### Requirement: Preview can follow an editor
A Preview pane's source MAY be a Preview pane in Edit mode in addition to a Files pane. A follower SHALL show the editor's current buffer as specified in `text-editor` and SHALL stay on that source until it is changed from the pane header, the editor pane closes (then it reverts to following the most recently focused Files pane), or it is pinned to another pane. A Preview pane SHALL NOT be its own source, and a follower SHALL NOT itself be used as the source of another pane. `pane.preview.source` SHALL accept an Edit-mode Preview pane id; it SHALL refuse ids that are not a Files pane or an Edit-mode Preview, that name the pane itself, or that name a follower.

#### Scenario: Pick the editor as source
- **WHEN** a Preview pane's source is set to an editor Preview pane
- **THEN** it renders that editor's buffer and updates as the text changes

#### Scenario: Cycle refused
- **WHEN** `pane.preview.source` is given the pane's own id, or the id of a pane that follows it
- **THEN** it returns an error and the source is unchanged

#### Scenario: Source stops being an editor
- **WHEN** the source pane leaves Edit mode
- **THEN** the follower keeps showing that pane's file as a normal preview of the same content

#### Scenario: Bad source kind
- **WHEN** `pane.preview.source` is given a Terminal pane
- **THEN** it returns an error and the source is unchanged

### Requirement: One toolbar layout for every preview and Edit mode
Every Preview pane SHALL use the same toolbar layout, whatever it shows (text, structured data, Markdown, SVG, images, PDFs, tables, logs, folders, cards, diffs) and in Edit mode: the source picker (or, in Edit mode, the mode and unsaved marker) on the left, and at the right edge, always in this order and at the same place, Wrap, Find and the Edit/Preview switch (a pencil that enters Edit mode, an eye in Edit mode that returns to the rendered preview). These three SHALL always be shown, disabled with an explanatory tooltip where they do not apply, so they never move. The view's own tools (Source for Markdown, SVG and tables; Pretty; Diff; image zoom and rotation; SVG background; PDF page controls; table options; folder thumbnail options; Reload; in Edit mode Live preview, Go to line, Line numbers and Open in External Editor) SHALL sit left of them as painted icons with a tooltip and an accessible name, and SHALL fold into the View options menu as labelled toggles when the pane is too narrow. Save (Edit mode) and Fill & Sign (PDF) SHALL never fold. The left side SHALL be clipped rather than overlap the tools. Diff SHALL be a toggle: a second click returns to the file view.

#### Scenario: Same spot in preview and Edit mode
- **WHEN** the user switches a Markdown preview to Edit mode and back
- **THEN** Wrap, Find and the switch stay at the same place on the bar, the switch showing a pencil in the preview and an eye in Edit mode

#### Scenario: Image preview
- **WHEN** an image is previewed
- **THEN** Wrap, Find and Edit are shown disabled at their usual place, and zoom and rotation sit to their left

#### Scenario: Narrow pane
- **WHEN** the pane is too narrow for the view's tools
- **THEN** they move into the View options menu while Wrap, Find and the switch stay on the bar

### Requirement: Preview and editor scroll alike
One mouse-wheel notch SHALL scroll the rendered preview and Edit mode the same distance, scaled by the Scroll speed preference (Preferences › Preview, 50–300 %, default 100 %, key `scroll_speed_pct`; out-of-range values are refused by `prefs.set` and clamped when read). Edit mode SHALL scroll smoothly by pixels, not snap to whole lines. Scroll bars in both SHALL be slim, rounded handles that stay faintly visible at rest and widen and strengthen under the pointer or while dragged, in theme colours.

#### Scenario: Faster scrolling
- **WHEN** the user sets Scroll speed to 200 %
- **THEN** a wheel notch scrolls twice as far in previews and in Edit mode

#### Scenario: Bad value
- **WHEN** `prefs.set` is given `scroll_speed_pct` 10 or "fast"
- **THEN** it returns an error and the preference is unchanged

## MODIFIED Requirements

### Requirement: Preview has pane-local Find
`Ctrl+F`, the Preview menu and the toolbar Find button SHALL open/focus Find in the
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
- **WHEN** Ctrl+F or the toolbar Find button is used in a log preview
- **THEN** the existing record-filter panel opens (the button toggles it and shows it as active) and its search field gains focus, retaining severity/time/regex controls and applied filters
- **AND** logs have no separate Filter button
- **AND** Escape hides that panel without clearing its filters
- **WHEN** Find is used in a table preview
- **THEN** the table's quick filter gains focus
- **WHEN** the Preview is an image, video, folder, binary, error or multi-file card grid
- **THEN** Find is disabled with an explanation rather than implying unavailable text/OCR/byte search
- **AND** Files Ctrl+F, terminal input, dialogs and query-field Ctrl+A/Ctrl+C retain their existing behavior

The `preview.find.{open,close,set,next,previous}` commands SHALL validate their
parameters and remain pane-local; `set` accepts query/case/word/regex options and
the other commands accept only an optional pane.

The Find control SHALL share the main source/title toolbar row (preview zoom is in the pane header, see `panes` "Pane zoom"),
not consume a separate row. At narrow widths view controls SHALL move into the
existing View options overflow menu; the source picker SHALL be clipped to keep that
menu and Wrap, Find and the Edit/Preview switch visible. An open search/filter panel MAY appear
below the toolbar.
