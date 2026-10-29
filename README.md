<p align="center"><img src="packaging/icon.png" width="128" alt="FileManagerUltra logo"></p>

# FileManagerUltra

A cross-platform (Linux · macOS · Windows), multi-pane file manager written in **Rust** with a native `eframe`/egui UI — no Electron, no webview.

- **Unlimited split panes**, each with its own file tree and file listing
- **Local drives, mounted shares, in-app SMB and SSH/SFTP** behind one `Vfs` trait
- **Drag & drop between panes** — drop onto a tree node or the listing; Ctrl = copy, Shift = move
- **File operations as background jobs** with progress, conflict handling and cancel
- **Trash with undo**, **live directory updates**, commander-style shortcuts (F5–F8, Tab)
- **Content-fitted, elastic file columns** with smooth divider resizing and remembered widths
- **Terminal panes** with application mouse support (mc/btop), resize reflow, multi-screen selection and configurable 5,000–1,000,000-row scrollback; hold Shift for local selection and scrolling
- **Rich Markdown previews** with nested lists, tasks, syntax-highlighted code and aligned tables; select/copy loaded text and zoom individual previews from 50% to 400%
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
