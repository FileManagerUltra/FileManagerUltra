# AGENTS.md: guide for AI agents and contributors

**FileManagerUltra** is a cross-platform multi-pane file manager written in **Rust only** (no JavaScript/TypeScript UI): native `eframe`/egui, no Tauri, no Electron, no webview. The workflow follows [PhotoCraft](https://github.com/storytold/photocraft): a layered Cargo workspace, everything user-visible is a command, tests are the gate, and sessions end with a green tree plus a `log/devlog.md` entry. Read this file first, then `docs/architecture.md`.

## 1. Orientation (5 minutes)

| Read | Why |
|---|---|
| `docs/architecture.md` | Crate map, layers, `Vfs`/`Session` model, concurrency lanes, the drag & drop contract |
| `docs/roadmap.md` | The five OpenSpec changes, current focus, what's next |
| `docs/development.md` | Build, test, run, system dependencies, platform quirks |
| `docs/contributing.md` | Style, commits, the "add a command" checklist |
| `openspec/specs/` | Approved specifications (what the product must do) |
| `openspec/changes/` | In-flight proposals: `proposal.md`, `specs/`, `design.md`, `tasks.md` |
| `log/devlog.md` | What actually landed, what is still open |

Work is driven by **OpenSpec**: propose with `/opsx-propose`, implement with `/opsx-apply`, archive with `/opsx-archive` (which syncs the change's deltas into `openspec/specs/`). Do not start work that is not covered by an approved change or an existing spec; if the spec is missing, update it in the same change.

## 2. Workspace map

```text
crates/
  brand        L0  product name/ids — the ONLY place branding strings live
  vfs          L0  VfsUri, Entry, Vfs trait (pure data, no I/O)
  fs-local     L0  local + mounted volumes
  fs-smb       L0  SMB (pure-Rust `smb` crate, same code on every OS)
  fs-sftp      L0  SSH/SFTP (russh + russh-sftp)
  testkit      L0  in-memory Vfs, temp-tree builders, command fuzzing
  ops          L1  copy/move/delete/mkdir/rename pipelines, conflicts, progress
  layout       L2  pane tree, per-pane history/selection/state
  engine       L3  Session, command registry, job scheduler, watcher, dir cache
  ui           L4  egui widgets (tree, virtualized listing, dialogs, theme)
apps/
  fm-app       L5  eframe shell: window, menus, shortcuts, wiring
xtask/                layers | brand-check | ci | bench
```

**Layering is enforced by `cargo xtask layers`.** A crate may depend only on equal-or-lower layers. A new crate must be registered in `xtask/src/layers.rs`. Nothing below L4 names an egui type; nothing above L3 performs filesystem I/O except through `engine`.

## 3. Golden rules

1. **Never crash.** No `unwrap()`, `expect()`, `panic!`, `todo!`, `unimplemented!`, `unreachable!` in non-test code — return `FmError` and propagate with `?`. A vanished file, a dead share, a full disk or a 100 000-entry directory must produce an actionable error. Fix a crash before building on top of it.
2. **No `unsafe`.** Workspace sets `unsafe_code = "forbid"`. There is no FFI; every backend is pure Rust.
3. **Everything is a command.** New behaviour = `CommandSpec` in `crates/engine/src/<area>_cmds.rs` + a `specs()` list extended in `commands.rs`, with id, label, menu path, shortcut, params doc, `enabled`, `run` and tests. The UI, shortcuts and (later) the control channel all dispatch by id. Never call `fm-ops` or a backend from `fm-ui`/`fm-app`.
4. **The UI thread never blocks.** All filesystem work goes through `engine`'s job lanes (`local`/`smb`/`sftp`). If your change makes a frame wait on I/O, it is wrong.
5. **Thin, data-driven UI.** View state lives in `fm-layout`'s pane state; colours come from `fm-ui/src/theme.rs` semantic tokens (`Tokens::dark()` / `Tokens::light()`, both exhaustive), never hard-coded — no `Color32::` literals outside `theme.rs`, and every widget must render correctly in both dark and light.
6. **Tests are the gate.** Every change ships tests: backends round-trip on tempdirs, `ops` covers progress/cancel/conflicts, `layout` covers split invariants, `engine` fuzzes every command with malformed params (must return `Err`, not panic), `ui` snapshots with `egui_kittest`.
7. **Names are not hard-coded.** Window title, About, installers and desktop entries read `fm-brand`. Rename with `cargo xtask rename "NewName"`; `cargo xtask brand-check` fails the build if a stale name survives.
8. **Hostile input.** Filenames, remote listings, drop payloads and command params are untrusted: bounds-check, use `get()` instead of indexing, cap allocation sizes. `VfsUri` parsing and command dispatch reject oversized/deep input before any work happens.
9. **Security is tested, not assumed.** Secrets never touch disk (`Prefs` has no secret fields), never appear in `FmError::Display` (redaction has tests), and state files are owner-only (`0600`/`0700`). Never introduce a code path that logs or formats a credential. The `security` OpenSpec capability is the checklist — extend it in the same change that adds the behaviour (credentials, host keys, symlink rules).

## 4. Before you finish a task

```sh
cargo test  -p <crates you touched>
cargo clippy -p <crates> --all-targets -- -D warnings
cargo xtask layers
cargo xtask brand-check
cargo fmt --all -- --check
```

Then append a terse entry to `log/devlog.md`: what landed, numbers (rows virtualized, copy throughput, test counts), what is still open. Sessions can end abruptly — the dev log plus a green tree is how the next agent picks up. Keep the tree building at every step.

If you added a command, also verify its shortcut/menu wiring and add a graceful-failure test (bad params → `Err`).

## 5. Parallel agents

- Use your own target dir (`CARGO_TARGET_DIR=target/agent-<name>`) to avoid the Cargo lock, and edit only files you own. Shared files (`engine/src/commands.rs`, `fm-app` menu/shortcut tables) get small surgical edits; re-read before editing.
- Put new commands in a **new module** (`engine/src/<area>_cmds.rs`) rather than growing an existing one.
- Keep every `Cargo.toml` valid at all times; write manifests atomically (temp file, then `mv`).

## 6. Where things are tracked

- `docs/roadmap.md` — milestones, current focus.
- `openspec/specs/` — approved behaviour specs (source of truth for "what the product must do").
- `openspec/changes/` — in-flight proposals and their task lists; archive when done (`/opsx-archive`).
- `docs/architecture.md` — structural decisions; update it in the same change that changes structure.
- `log/devlog.md` — the running engineering log (local, gitignored).

## 7. Dependencies to know about

- `smb` (SMB) is used in its blocking mode and needs every opened handle closed explicitly → SMB work stays on the `smb` lane (2 workers); never call it from the UI thread. `sspi` is vendored in `vendor/` (rc crypto pins relaxed to coexist with `russh`).
- `russh-sftp` is async → the `fs-sftp` crate owns a tokio runtime and blocks on it inside its `Vfs` impl; no other crate depends on tokio directly.
- `trash` handles XDG/Recycle Bin/macOS trash — deletes must go through `ops::trash`, never `fs::remove_dir_all`.
- `notify` events are debounced and only invalidate `DirCache`; they never call `read_dir` inline.
