## Context

Delimited text currently goes through `Kind::Text("csv")` and the 1 MiB text preview.
The samples that motivated this change (`tests/csv/`, not committed) show what real
files look like: `,`, `;` and tab delimiters; quoted fields with HTML and embedded
newlines (a 16 MB file with 25 745 lines but far fewer records); a UTF-8 BOM; mixed
CRLF/LF; a TSV without a header row whose first column is numeric. The log viewer
(`fm_preview::log`, `LogView`) and the PDF service are the patterns to follow: a pure
model in `fm-preview`, a live view in the engine, commands for every action and a
thin UI.

## Goals / Non-Goals

**Goals:** open any size of delimited file quickly on every backend; exact rows for
quoted multi-line records; sort and filter without blocking the UI; readable long
cells; every action a command.

**Non-Goals:** editing, formulas, statistics, `.xlsx`, joining files, persisting view
settings across files.

## Decisions

### D1. One byte-oriented, quote-aware scanner
Records are found by a state machine over bytes, not by lines: a quote opens a quoted
field only at the start of a field, `""` inside a quoted field is an escaped quote, a
quote followed by anything else closes it (lenient, like spreadsheet apps), and an
unquoted record ends at `\n` (a trailing `\r` is trimmed). `memchr` finds the next
interesting byte in each state, so plain data is skipped at memory speed. A quote in
the middle of an unquoted field (`27" screen`) is ordinary text, which keeps one stray
quote from swallowing the rest of the file.
*Alternative:* the `csv` crate. Rejected: it parses records but does not expose
resumable byte offsets for an index over a source read in blocks, and it would be a
new dependency for something small.

### D2. Text is UTF-8 inside the table model
UTF-8 is scanned as is (a BOM is skipped by starting the data after it). UTF-16 with a
BOM is transcoded to UTF-8 once, in memory, capped at the memory limit (larger UTF-16
files show the first 256 MiB with a note). Fields are decoded with lossy UTF-8.

### D3. Sources: memory up to 256 MiB, blocks beyond
`ByteSource` has `len()` and `read_at(offset, len)`. Files up to 256 MiB are read once
into an `Arc<[u8]>`; larger files use a `VfsSource` that reads 1 MiB blocks through
`Vfs::open_read_at` with a small LRU (16 blocks). Both work for local, SMB and SFTP.
All reads happen on the document's own threads (D5), never on the UI thread.

### D4. A sparse index of record offsets
The index stores the byte offset of every 32nd record (`STRIDE`), so 50 million rows
cost 12.5 MB. Row *r* is found by seeking to checkpoint `r / 32` and parsing forward at
most 31 records. Sorted and filtered views store `(offset, row number)` per row, so
their rows are read directly and the original row number stays visible.

### D5. Threads owned by the document, not the job lanes
A table opens with two threads of its own: the indexer (one sequential pass, then it
ends) and a reader that serves row requests and runs view jobs (filter + sort) in
between. Each has a cancel flag set when the view is dropped (another file selected,
pane closed). A newer view job replaces an older one at the next record boundary.
This mirrors Find Files and the PDF pool; a long index pass on a slow share must not
hold a job lane that file copies use.

### D6. Row cache and requests
The UI asks the view for the visible rows every frame. Present rows come from an LRU
of parsed rows (4 096 rows); missing ones are posted as one "wanted" range that
replaces the previous request, so scrolling fast skips rows that went out of view.
The reader wakes the UI (session waker) when rows arrive.

### D7. Sort keys and budget
Column types are inferred from a sample (the first 2 000 records): integer, decimal
(dot, or comma when the delimiter is `;`), or text. Numeric columns sort by value with
non-numbers after numbers; text sorts case-insensitively on the first 256 bytes of
the value; empty cells sort last in both directions. Up to three keys. A view job
refuses to sort when its keys would exceed 512 MiB or 20 million rows, and says so;
filtering alone stores only offsets and row numbers and is capped at 20 million rows.

### D8. Filter model
A quick filter (text, regex flag, one column or all) plus up to 16 column conditions,
all combined with AND. Text matching is case-insensitive. Regexes use the same size
limits as the log filter, and an invalid pattern is an error that leaves the current
view unchanged. Greater/less than compare numerically when both sides are numbers and
as text otherwise.

### D9. Own vertical scrolling in the grid
egui positions are `f32`; at 20 px per row they lose whole rows beyond about 800 000
rows. The grid keeps its own first-row index (`u64`) and draws a custom vertical
scrollbar; horizontal scrolling is a pixel offset over the visible columns. Only
visible columns are laid out, so a file with 2 000 columns costs no more than the
columns on screen.

### D10. View state
Sort, filter, dialect overrides, hidden columns, selection and the Record panel live
in the engine's `TableView` and change only through commands. Column widths and the
scroll position are presentation state in egui memory, like the other preview views.
Changing the delimiter or header choice re-opens the document with the override.

## Risks / Trade-offs

- [A malformed file with an unclosed quote] → a record longer than 1 MiB is cut at the
  next newline after the cap and counted; the toolbar notes "N malformed records".
- [Huge rows (thousands of columns, megabyte cells)] → at most 16 384 fields per record
  and 64 KiB per displayed field (the Record panel says when a value was cut).
- [Slow shares] → the indexer reports progress and the visible rows still arrive first
  because the reader serves row requests before view-job work.
- [Detection guesses wrong] → the delimiter and header overrides are one click away.
