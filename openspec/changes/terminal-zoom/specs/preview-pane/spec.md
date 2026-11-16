## MODIFIED Requirements

### Requirement: Preview zoom reset takes precedence over workspace switching
Ctrl+0 (Cmd+0 on macOS) SHALL reset only the focused Preview's zoom when no dialog or
text field owns the keyboard. It SHALL NOT switch
workspaces or change interface zoom. Ctrl+Shift+0 SHALL reset the interface zoom,
as in every other pane. Outside Preview, Ctrl+0 SHALL retain workspace 10
selection; terminal input and text fields SHALL keep their existing ownership.

#### Scenario: Reset zoom from Preview
- **WHEN** Preview is focused with no dialog or text field owning the keyboard and the user presses Ctrl+0
- **THEN** only that Preview's zoom resets
- **AND** the workspace and interface zoom do not change

### Requirement: Preview has pane-local Find
`Ctrl+F`, the Preview menu and a toolbar Find button (non-log views) SHALL open/focus Find in the
focused Preview, without opening the Files-pane filter. Text, structured text
(including raw/pretty mode) and rendered Markdown SHALL offer a query field,
case-sensitive, whole-word and regular-expression options, a current/total match
counter, Previous/Next buttons, and distinct current/other match highlighting in
both themes. Searches SHALL use already-loaded content only, not read files or
search hidden Markdown syntax. Text is searched per source line; Markdown is
searched per rendered paragraph/heading, code block or table cell.

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

### Requirement: Pane-local preview magnification
Every Preview body SHALL support independent 50%-400% magnification in 20% steps
through `preview.scale.in`, `preview.scale.out` and `preview.scale.reset`.
The pane header's zoom (`panes` requirement "Pane zoom", which dispatches
`pane.scale.*` and routes it to these commands) SHALL offer zoom in, the
percentage/reset and zoom out for every view type, including text, Markdown,
structured text, logs, images, hex, metadata, folder summaries, thumbnails,
multi-file cards and video; the Preview toolbar SHALL NOT repeat them.
Only that pane's content SHALL scale; pane chrome, other panes and the application
zoom preference SHALL remain unchanged. Scale SHALL survive changes of previewed
file within the same pane. Images SHALL retain Fit/Actual modes; choosing either
SHALL reset additional magnification. Zoom SHALL NOT increase file/decode limits.

A focused Preview SHALL accept Ctrl/Cmd+= (or +), Ctrl/Cmd+- and Ctrl/Cmd+0.
A shortcut SHALL do the same in every pane: Ctrl/Cmd+Shift+= (or +), Ctrl/Cmd+Shift+-
and Ctrl/Cmd+Shift+0 SHALL keep changing the interface zoom while a Preview has focus.
Dialogs, popups and focused text fields SHALL retain keyboard ownership.
Commands SHALL reject malformed parameters and invalid explicit panes.

#### Scenario: Zoom without changing the app
- **WHEN** a focused Preview is zoomed in, out or reset
- **THEN** its body changes size while the toolbar, Files panes, other previews,
  global zoom preference and egui zoom factor remain unchanged
- **AND** minimum/maximum scales are enforced and bad parameters return an error

#### Scenario: Interface zoom keys from a Preview
- **WHEN** a Preview has focus and the user presses Ctrl+Shift+=
- **THEN** the whole application zooms in one step, as from any other pane, and the Preview's own zoom is unchanged
