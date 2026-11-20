## 1. Table model (`fm-preview::table`)

- [x] 1.1 Dialect detection (delimiter, header) and encoding handling (BOM, UTF-16 transcoding)
- [x] 1.2 Quote-aware record scanner with record/field caps and malformed-record counting
- [x] 1.3 Record parser (fields, 64 KiB display cap, trailing `\r`)
- [x] 1.4 Byte sources: memory (≤ 256 MiB) and VFS blocks with LRU
- [x] 1.5 Sparse index builder (STRIDE 32) with cancel and progress
- [x] 1.6 Column type inference; sort keys and comparison; filter evaluation (quick filter, conditions, regex limits)
- [x] 1.7 `Kind::Table`, `Preview::Table(TableDoc)` with sample rows and raw text; sniffing `.csv`/`.tsv`/`.tab`; CSV association maps to the table
- [x] 1.8 Tests: the sample shapes (quoted newlines, BOM, CRLF/LF, `;`, TSV without header), stray quotes, unclosed quote, caps, sorting, filtering

## 2. Engine

- [x] 2.1 `TableView` with indexer and reader threads, cancel on drop, waker, row cache and wanted range
- [x] 2.2 View jobs (filter + sort) with progress, replacement and budgets
- [x] 2.3 `Body::Table` in the preview slot; Preview/Source toggle for tables
- [x] 2.4 `preview_table_cmds.rs`: sort, filter, condition add/remove, filter clear, dialect, select, copy, goto, column hide, show all, record panel; registered in `commands.rs`
- [x] 2.5 Tests: index/row access through the session, sort/filter end to end, cancel on file change, every command's bad-parameter test (and the global fuzz)

## 3. UI

- [x] 3.1 Virtualized grid with own vertical scrolling, sticky header and row numbers, column resize, selection, keyboard navigation
- [x] 3.2 Header sort indicators and header menu (sort, add condition, hide, show all, copy names)
- [x] 3.3 Table toolbar: delimiter/header choices, quick filter with column scope and Regex, condition chips, Record toggle, counts and progress, Go to row
- [x] 3.4 Record panel
- [x] 3.5 Snapshot tests in dark and light; interaction test for sorting and filtering

## 4. Finish

- [x] 4.1 Preferences filter hint no longer says CSV opens as text; app waker wiring
- [x] 4.2 CHANGELOG, docs/architecture.md (table threads), devlog
- [x] 4.3 `cargo test` for touched crates, clippy, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --check`
