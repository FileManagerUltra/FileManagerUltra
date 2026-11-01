# FileManagerUltra

A cross-platform (Linux · macOS · Windows), multi-pane file manager written in **Rust** with a native `eframe`/egui UI — no Electron, no webview.

- **Unlimited split panes**, each with its own file tree and file listing
- **Local drives, mounted shares, in-app SMB and SSH/SFTP** behind one `Vfs` trait
- **Drag & drop between panes** — drop onto a tree node or the listing; Ctrl = copy, Shift = move
- **File operations as background jobs** with progress, conflict handling and cancel
- **Trash with undo**, **live directory updates**, commander-style shortcuts (F5–F8, Tab)
- **Content-fitted, elastic file columns** with smooth divider resizing and remembered widths
- **Local file times** using the operating system's timezone and daylight-saving rules
- **Terminal panes** with application mouse support (mc/btop), resize reflow, multi-screen selection and configurable 5,000–1,000,000-row scrollback; hold Shift for local selection and scrolling
- **Rich Markdown previews** with nested lists, tasks, syntax-highlighted code, per-block copy buttons and aligned tables; select/copy loaded text without UI chrome and zoom individual previews from 50% to 400%
- **Open in Preview Panel** from file and thumbnail context menus, reusing an existing Preview or creating one when needed
- **Preview Find (Ctrl+F)** with match navigation/highlighting, case, whole-word and regex options for text/Markdown; logs retain their record filters
- **Git status** in listings and the tree (tinted names, gutter letters, folder dots, branch in the status bar) and a **Diff view in the Preview pane** (modified files open as a diff against HEAD or any branch/tag, word highlights, F7/Shift+F7 between changes); read-only, pure-Rust `gix`, nothing runs from repository config
- **Folder context menus** on empty listing space and tree folders, with creation, paste, refresh, bookmarking and properties actions
- **Log previews** with Live tail, search/severity filters and UTC time ranges: quick presets from 5 minutes to 7 days, or custom dates and times

## Status

Pre-alpha. Work is planned and tracked with [OpenSpec](./openspec/) — see `docs/roadmap.md` for the five changes; `AGENTS.md` is the contributor/agent guide; `docs/architecture.md` describes the design.

```sh
cargo run -p fm-app      # run the native app
cargo test  --workspace
cargo xtask layers       # layering enforcement
```

## Install

See [docs/install.md](docs/install.md).

## License

MIT — see [LICENSE](LICENSE). Logo source: `packaging/logo.svg`. Bundled Atkinson Hyperlegible fonts use the [SIL Open Font License](crates/ui/assets/fonts/OFL.txt).
