<p align="center">
  <img src="packaging/logo.svg" alt="FileManagerUltra logo" width="96">
</p>

<h1 align="center">FileManagerUltra</h1>

<p align="center">
  A multi-pane file manager written in Rust, with as many panes as you need.<br>
  Linux · macOS · Windows. Native <code>eframe</code>/egui UI, no Electron, no webview.
</p>

<p align="center">
  <a href="https://filemanagerultra.com/">Website</a> ·
  <a href="https://github.com/FileManagerUltra/FileManagerUltra/releases">Download</a> ·
  <a href="docs/install.md">Install guide</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/hero-dark.webp">
  <img src="docs/screenshots/hero-light.webp" alt="FileManagerUltra with several panes: file trees, listings, a terminal and a preview">
</picture>

> **Status: pre-alpha.** Expect rough edges and changes between releases. Installers are not signed yet, so your OS will warn on first start.

## Highlights

- **Unlimited split panes.** Split right or down as often as you like. Each pane has its own file tree, listing, history and selection.
- **Local, SMB and SFTP.** Local drives, mounted shares, in-app SMB and SSH/SFTP sit behind one virtual filesystem, so every feature works the same everywhere. All backends are pure Rust.
- **Drag & drop between panes.** Drop onto a tree node or the listing. Ctrl copies, Shift moves.
- **Background jobs.** Copy, move and delete run with progress, conflict handling and cancel. The UI never blocks on I/O.
- **Trash with undo** and **live directory updates**.
- **Commander-style shortcuts.** F5–F8 for copy, move, new folder and delete; Tab swaps panes. Every action is a command with a searchable shortcut reference in Preferences.
- **Workspaces.** Keep different layouts in tabs, save them and reopen them later.
- **Dark and light themes** with a custom accent colour. Local file times follow your operating system's timezone.

## Previews

Open any file in a Preview pane next to the listing, from the context menu or the thumbnail grid. Press Ctrl+F to search inside a preview.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/diff-dark.webp">
  <img src="docs/screenshots/diff-light.webp" alt="Side-by-side Git diff in the Preview pane">
</picture>

- **Git status and diff.** Tinted names, status letters, folder dots and the branch in the status bar. Modified files open as a side-by-side diff against HEAD or any branch or tag, with word highlights and F7 / Shift+F7 to jump between changes. Read-only, pure-Rust `gix`; nothing runs from repository config.
- **Markdown** with nested lists, tasks, aligned tables and syntax-highlighted code with copy buttons. Links are clickable, and a toggle shows the source. Zoom from 50% to 400%.
- **Images and thumbnails.** Preview one image, or switch to a thumbnail grid of the folder. Include subfolders, group files under a heading per folder, and sort by name, modified date or type.
- **Logs** with live tail, search, severity filters and UTC time ranges from 5 minutes to 7 days.

<table>
  <tr>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/markdown-dark.webp">
        <img src="docs/screenshots/markdown-light.webp" alt="Markdown preview">
      </picture>
    </td>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/thumbs-dark.webp">
        <img src="docs/screenshots/thumbs-light.webp" alt="Thumbnail grid preview">
      </picture>
    </td>
  </tr>
  <tr>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/log-dark.webp">
        <img src="docs/screenshots/log-light.webp" alt="Log preview with filters">
      </picture>
    </td>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/terminal-dark.webp">
        <img src="docs/screenshots/terminal-light.webp" alt="A terminal pane next to a file listing">
      </picture>
    </td>
  </tr>
</table>

## Terminals are panes

A terminal sits beside your files and runs any command-line tool, including full-screen TUIs such as `mc` and `btop`, with application mouse support, resize reflow, multi-screen selection and 5,000–1,000,000 rows of scrollback. Hold Shift for local selection and scrolling when an application captures the mouse. Ctrl+click (Cmd+click on macOS) opens a web link.

That makes it a good place for a terminal coding agent: run it in one pane, keep the project in a Files pane, and review every change in a Preview pane with Git status markers and a diff against HEAD.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/ai-workflow-dark.webp">
  <img src="docs/screenshots/ai-workflow-light.webp" alt="A coding agent in a terminal pane, the project files and a diff in one window">
</picture>

## SFTP, SMB and the network

Point any pane at `sftp://user@host/…` or `smb://server/share`. Remote folders behave like local ones: split, drag & drop, and copy or move as background jobs. Connect with a password or key file, and save the server. SFTP host keys are confirmed on first use and a changed key is refused. The Network pane lists file-sharing and remote-login hosts found on your LAN.

<table>
  <tr>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/sftp-dark.webp">
        <img src="docs/screenshots/sftp-light.webp" alt="An SFTP pane next to a local pane">
      </picture>
    </td>
    <td>
      <picture>
        <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/network-dark.webp">
        <img src="docs/screenshots/network-light.webp" alt="The Network pane listing hosts on the LAN">
      </picture>
    </td>
  </tr>
</table>

## Install

Installers for Windows (x64), macOS (Apple silicon) and Linux (`.deb` and `.AppImage`) are attached to each [GitHub Release](https://github.com/FileManagerUltra/FileManagerUltra/releases). Each download is a zip: unzip it first. See [docs/install.md](docs/install.md) for the first-start steps on each OS.

Or build from source with a recent stable Rust toolchain:

```sh
cargo build --release --locked -p fm-app
./target/release/filemanagerultra
```

The app checks GitHub Releases for newer versions at startup. You can turn this off in Preferences; updating is always manual.

## Security

Filenames, remote listings, drop payloads and command parameters are treated as untrusted input. Secrets never touch disk and never appear in error messages, state files are owner-only, and deletes go through the system trash. The checklist lives in [`openspec/specs/security`](openspec/specs/security/spec.md).

## Built to be trusted

- **Never crash:** no `unwrap()`/`panic!` in non-test code, and `unsafe` is forbidden workspace-wide. A vanished file, a dead share or a 100 000-entry folder produces an actionable error.
- **Everything is a command:** the UI, menus and shortcuts all dispatch the same registered commands, which are fuzzed with malformed parameters.
- **Layered workspace:** crates may only depend on equal or lower layers, enforced by `cargo xtask layers`.

## Contributing

Work is planned with [OpenSpec](openspec/): approved behaviour lives in `openspec/specs/`, in-flight changes in `openspec/changes/`.

| Read | For |
|---|---|
| [AGENTS.md](AGENTS.md) | Contributor and agent guide, golden rules |
| [docs/architecture.md](docs/architecture.md) | Crate map, layers, concurrency, drag & drop contract |
| [docs/development.md](docs/development.md) | Build, test, run, platform notes |
| [docs/contributing.md](docs/contributing.md) | Style, commits, how to add a command |
| [docs/roadmap.md](docs/roadmap.md) | What is planned and what is next |

```sh
cargo run -p fm-app      # run the native app
cargo test --workspace
cargo xtask ci           # everything CI runs, locally
```

## License

Apache License 2.0, see [LICENSE](LICENSE). Logo source: `packaging/logo.svg`. Bundled Atkinson Hyperlegible fonts use the [SIL Open Font License](crates/ui/assets/fonts/OFL.txt).
