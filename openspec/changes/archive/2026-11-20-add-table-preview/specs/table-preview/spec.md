## ADDED Requirements

### Requirement: Delimited text is shown as a table
Files associated with CSV (by default `*.csv`, `*.tsv` and `*.tab`) SHALL open in the
Preview pane as a table with a sticky header row, a row-number column, columns sized
from their content and resizable by dragging the header edge, numeric columns
right-aligned, empty cells muted, and the selected cell and its row highlighted. A cell
whose value spans several lines SHALL show its first line followed by a line-break
marker. The grid SHALL lay out only the visible rows and columns and SHALL address rows
exactly for any row count, including counts beyond 10 million. The toolbar SHALL show
the row and column counts. The raw text SHALL be available with the existing
Preview/Source toggle.

#### Scenario: Shop export with quoted multi-line fields
- **WHEN** a `,`-delimited file whose quoted description fields contain HTML and line breaks is previewed
- **THEN** each record is one table row, the line breaks inside quotes do not start new rows, and the row count equals the number of records

#### Scenario: Wide file
- **WHEN** a file with 2 000 columns is previewed and scrolled sideways
- **THEN** the header stays aligned with its columns and only the visible columns are drawn

#### Scenario: Raw text
- **WHEN** the user switches the Preview/Source toggle
- **THEN** the file's text is shown as in the text preview, and switching back restores the table with its sort and filter

### Requirement: Dialect and header are detected and can be overridden
The delimiter SHALL be detected from the content among comma, semicolon, tab and `|`
(`.tsv` and `.tab` default to tab), with `"` as the quote character: a quote opens a
quoted field only at the start of a field, `""` inside it is a literal quote, and a
quote inside an unquoted field is ordinary text. Records end at `\n` outside quotes,
with a trailing `\r` removed. Whether the first row is a header SHALL be detected (a
first row of distinct non-numeric names above rows with numeric values in the same
columns is a header; a first row like the rows below it is data). Without a header,
columns SHALL be named "Column 1", "Column 2"…; rows with more fields than the header
SHALL get extra named columns. UTF-8 with or without BOM and UTF-16 with BOM SHALL be
read; invalid UTF-8 SHALL be shown with replacement characters. The toolbar SHALL let
the user choose the delimiter (Auto, comma, semicolon, tab, `|`) and the header (Auto,
first row is header, no header); a choice applies to the shown file and is forgotten
when another file is selected.

#### Scenario: Semicolon export with BOM and CRLF
- **WHEN** a `;`-delimited UTF-8 file with a BOM and mixed CRLF/LF line endings is previewed
- **THEN** the delimiter is semicolon, the first header name has no BOM character, and no cell ends in a carriage return

#### Scenario: TSV without a header
- **WHEN** a tab-separated file whose first row holds an id, a name and two numbers is previewed
- **THEN** the first row is shown as data and the columns are named Column 1 to Column 4

#### Scenario: Override the delimiter
- **WHEN** the user picks Semicolon for a file detected as comma-separated
- **THEN** the table is re-read with semicolons and the sort and filter are cleared

### Requirement: Large files open quickly and stay responsive
A table SHALL show its first rows without waiting for the whole file. The file SHALL be
indexed in the background with visible progress, and the row count SHALL grow while
indexing runs. Only visible rows SHALL be parsed. Files up to 256 MiB SHALL be held in
memory; larger files SHALL be read in blocks on demand so memory use does not grow
with the file size, on local, SMB and SFTP alike. The UI thread SHALL never wait for
file reads, indexing, filtering or sorting. Selecting another file or closing the pane
SHALL stop the document's background work.

#### Scenario: Multi-gigabyte file
- **WHEN** a 2 GB CSV file is previewed
- **THEN** the first rows appear before indexing finishes, the toolbar shows indexing progress, and scrolling to any indexed row shows it without reading the file from the start

#### Scenario: Leave during indexing
- **WHEN** the user selects another file while a large table is still indexing
- **THEN** the indexing thread ends and its memory is released

### Requirement: Sorting by columns
Clicking a column header SHALL sort by that column ascending, a second click
descending, a third click SHALL return to file order. Shift-click SHALL add the column
as a further sort key, up to three keys. The header SHALL show the direction and, with
several keys, their order. Columns whose sampled values are numbers SHALL sort by
numeric value (a decimal comma counts as a decimal point in `;`-delimited files), with
non-numeric values after the numbers; other columns SHALL sort as case-insensitive
text. Empty cells SHALL sort last in both directions. Sorting SHALL run off the UI
thread with progress, apply to the filtered rows when a filter is active, keep each
row's original row number visible, and be refused with an explanation when it would
exceed the sort budget.

#### Scenario: Numeric sort
- **WHEN** the user sorts a price column holding 9.5, 10 and 100 ascending
- **THEN** the order is 9.5, 10, 100, not 10, 100, 9.5

#### Scenario: Back to file order
- **WHEN** the user clicks a sorted-descending header
- **THEN** the rows return to their order in the file

### Requirement: Filtering rows
The toolbar SHALL offer a quick filter that keeps the rows where any column (or one
chosen column) contains the typed text, case-insensitively, or matches it as a regular
expression when Regex is on. Column conditions SHALL be added from a column header's
menu with an operator (contains, does not contain, equals, does not equal, greater
than, less than, is empty, is not empty) and a value; up to 16 conditions SHALL combine
with the quick filter using AND and show as chips that remove their condition. Greater
and less than SHALL compare numerically when both values are numbers and as text
otherwise. The toolbar SHALL show how many rows match. Filtering SHALL run off the UI
thread, a newer filter SHALL replace a running one, and an invalid regular expression
SHALL show an error and keep the previous rows.

#### Scenario: Quick filter on one column
- **WHEN** the user types `NL` with the quick filter limited to the Country column
- **THEN** only rows whose Country contains "nl" in any case remain, and the toolbar shows the match count

#### Scenario: Condition chips
- **WHEN** the user adds "Rate greater than 20" and then removes its chip
- **THEN** only rows with a rate above 20 show while the chip exists, and all rows show again after it is removed

#### Scenario: Invalid pattern
- **WHEN** Regex is on and the quick filter is `([a-z`
- **THEN** an error is shown and the rows shown before are unchanged

### Requirement: Reading and copying cells
Clicking a cell SHALL select it; arrow keys, Page Up/Down and Home/End SHALL move the
selection when the table has keyboard focus, scrolling it into view. A Record panel
SHALL show the selected row as column name and value pairs with full, wrapped,
selectable values, and SHALL say when a value was cut at the field cap. Copy cell
(Ctrl+C in the table), copy row (tab-separated) and copy column names SHALL put text on
the clipboard. Go to row SHALL scroll to and select a row number. Columns SHALL be
hidden from the header menu and shown again with Show all columns.

#### Scenario: Read a long HTML description
- **WHEN** the user selects a row and opens the Record panel
- **THEN** the description field shows in full, wrapped to the panel width, and can be selected and copied

#### Scenario: Go to row
- **WHEN** the user runs Go to row with 120 000 on a file with 500 000 rows
- **THEN** row 120 000 is selected and visible

### Requirement: Table commands
The table actions SHALL be commands: `preview.table.sort`, `preview.table.filter`,
`preview.table.condition.add`, `preview.table.condition.remove`,
`preview.table.filter.clear`, `preview.table.dialect`, `preview.table.select`,
`preview.table.copy`, `preview.table.goto`, `preview.table.column.hide`,
`preview.table.columns.show_all` and `preview.table.record`, with ids, labels, menu paths and params
docs, enabled only when the focused or given pane shows a table. Malformed parameters
(unknown columns, rows beyond the table, unknown operators or delimiters, oversized
text, more than 16 conditions) SHALL return an error without changing the view.

#### Scenario: Unknown column
- **WHEN** `preview.table.sort` is given a column index beyond the last column
- **THEN** it returns an error and the sort is unchanged
