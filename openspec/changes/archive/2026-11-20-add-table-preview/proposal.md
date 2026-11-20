## Why

CSV and TSV files open as plain text in the Preview pane today. Real exports (shop
catalogues, cloud inventories, query results) are wide, use `;` or tabs, quote fields
that contain newlines and HTML, and run to millions of rows, so as text they are
unreadable and as a whole they are slow to open elsewhere. A table view with sorting
and filtering answers "what is in this export?" without leaving the file manager, and
it has to stay fast on files far larger than memory.

## What Changes

- **Table view for delimited text.** `.csv`, `.tsv` and `.tab` files (and anything
  the user associates with CSV in Preferences) open as a table: a sticky header, row
  numbers, auto-sized and resizable columns, right-aligned numbers, multi-line cells
  shown on one line with a marker, and the selected cell highlighted.
- **Detection with overrides.** Delimiter (`,` `;` tab `|`), quoting and whether the
  first row is a header are detected from the content; the user can override the
  delimiter and the header choice from the toolbar. UTF-8 (with or without BOM) and
  UTF-16 with BOM are read; invalid UTF-8 never breaks the view.
- **Large files.** The file is indexed in the background; rows appear at once and
  the row count grows while indexing runs. Only the visible rows are parsed. Files up
  to 256 MiB are held in memory; larger files are read in blocks on demand (local,
  SMB and SFTP alike), so a multi-gigabyte file opens without loading it whole.
- **Sorting.** Clicking a header sorts ascending, then descending, then back to file
  order; Shift-click adds secondary keys (up to three). Numeric columns sort by
  value (including decimal commas in `;` files); empty cells always sort last.
- **Filtering.** A quick filter matches text in all columns or one chosen column,
  optionally as a regular expression. Column conditions (contains, does not contain,
  equals, does not equal, greater than, less than, is empty, is not empty) combine
  with it; active conditions show as removable chips. Filtering and sorting run off
  the UI thread with progress and can be replaced by a newer request at any time.
- **Reading cells.** A Record panel shows the selected row as "column: value" pairs
  with full, wrapped, selectable values (for long HTML or multi-line fields).
  Copy cell, copy row (tab-separated) and Go to row are available; columns can be
  hidden and shown again. The raw text stays one toggle away.
- Everything is a command (`preview.table.*`) with bad-parameter tests.
- **Out of scope.** Editing and saving, formulas, charts, joining files, Excel
  (`.xlsx`) and other binary spreadsheet formats, and column statistics.

## Capabilities

### New Capabilities
- `table-preview`: detecting dialect and header, the background index and block
  reads for large files, the table view, sorting, filtering, the Record panel,
  copying, Go to row, hidden columns and the raw-text toggle.

### Modified Capabilities
- `preview-pane`: "Supported previews" gains delimited-text tables; "Hostile files are
  bounded" gains the table limits (record, field and column caps, sort budget);
  "Configurable preview filename filters" no longer says CSV opens as text.

## Impact

- `fm-preview` (L1): new `table` module (dialect detection, quote-aware scanner,
  record parser, byte sources, sort keys, filter evaluation). No threads, no egui.
- `fm-engine` (L3): `preview_table.rs` (the live view: index, row cache and view-job
  threads owned by the document) and `preview_table_cmds.rs` (commands); `Body::Table`
  in `preview_cmds.rs`; CSV associations map to the table kind.
- `fm-ui` (L4): new `preview_table.rs` with a custom virtualized grid (own vertical
  scrolling so row counts beyond f32 pixel precision stay exact), toolbar and Record
  panel; snapshot tests in dark and light.
- No new dependencies (`memchr` and `regex` are already in the workspace).
