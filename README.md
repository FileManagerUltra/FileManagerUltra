<p align="center"><img src="packaging/icon.png" width="128" alt="FileManagerUltra logo"></p>

# FileManagerUltra

A cross-platform (Linux · macOS · Windows), multi-pane file manager written in **Rust** with a native `eframe`/egui UI — no Electron, no webview.

- **Unlimited split panes**, each with its own file tree and file listing
- **Local drives, mounted shares, in-app SMB and SSH/SFTP** behind one `Vfs` trait
- **Drag & drop between panes** — drop onto a tree node or the listing; Ctrl = copy, Shift = move
- **File operations as background jobs** with progress, conflict handling and cancel
- **Trash with undo**, **live directory updates**, commander-style shortcuts (F5–F8, Tab)

## Status

Pre-alpha. Work is planned and tracked with [OpenSpec](./openspec/) — see `docs/roadmap.md` for the five changes; `AGENTS.md` is the contributor/agent guide; `docs/architecture.md` describes the design.

```sh
cargo run -p fm-app      # run the app (after change 1 scaffolds it)
cargo test  --workspace
cargo xtask layers       # layering enforcement
```

## Install

See [docs/install.md](docs/install.md).

## License

MIT — see [LICENSE](LICENSE). Logo source: `packaging/logo.svg`.
