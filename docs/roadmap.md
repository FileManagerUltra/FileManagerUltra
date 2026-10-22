# Roadmap

Five OpenSpec changes, in order. Each change proposes, implements and archives itself; its deltas sync into `openspec/specs/` so the specs always describe what the product actually does.

| # | Change | Delivers | Status |
|---|---|---|---|
| 1 | `scaffold-workspace` | Cargo workspace, crate skeletons, `xtask layers` + `brand-check`, AGENTS.md/docs/devlog, branding module, empty themed window, **security baseline (fuzzed URIs/params, redacted errors, owner-only state files, no `unsafe`)** | done |
| 2 | `multi-pane-ui` | Recursive split panes, per-pane file tree + virtualized listing, selection, navigation/history, commander shortcuts (F5–F8, Tab), **dark/light/system theme from day one** (tokens only, no hard-coded colours); pane code works against any `VfsUri`, so remote backends plug in without UI rework | current |
| 3 | `dnd-and-file-ops` | Cross-pane drag & drop (tree node + listing targets, Ctrl=copy/Shift=move), external file drop, `ops` pipelines, job queue with progress/cancel, conflict dialogs, trash + undo | next |
| 4 | `network-backends` | **SFTP client that fills a pane** (`sftp://…` → tree + listing, connect dialog, in-memory passwords/TOFU host keys) and SMB (pavao / Windows UNC), credentials profiles, space/free-space in the status bar | next |
| 5 | `live-watch-and-packaging` | `notify`-driven directory refresh, session persistence (panes/sizes/locations), `.deb`/AppImage, `.msi`, `.dmg` | next |

**Current focus:** change 2 — `multi-pane-ui` (propose with `/opsx-propose` first). Change 1's commit + `/opsx-archive scaffold-workspace` happen when the user asks.

**Security is a standing concern, not a change.** The `security` OpenSpec capability starts in change 1 (hostile input, secrets, state-file permissions, symlink policy, no `unsafe`) and each change that adds a behaviour area extends it in the same change: change 3 adds drag & drop / `ops` containment and overwrite rules; change 4 adds credentials, host keys and remote listing caps. A change that adds security-relevant behaviour without a spec + failing-test proof is not done.

Out of scope until after these five: archives, previews, tabs, full-text search, plugins, CLI/MCP automation (the command registry lands in change 1 so nothing has to be re-wired later).

## Working agreement

- Nothing is "done" until `cargo test`, `cargo clippy -D warnings`, `cargo xtask layers`, `cargo xtask brand-check` and `cargo fmt --check` are green, tests are added, and `log/devlog.md` has an entry.
- Scope cuts happen in the proposal, not mid-change: if a task turns out wrong, edit the change's `tasks.md`/`design.md` before continuing.
