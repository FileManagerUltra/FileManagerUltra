# Roadmap

Five OpenSpec changes, in order. Each change proposes, implements and archives itself; its deltas sync into `openspec/specs/` so the specs always describe what the product actually does.

| # | Change | Delivers | Status |
|---|---|---|---|
| 1 | `scaffold-workspace` | Cargo workspace, crate skeletons, `xtask layers` + `brand-check`, AGENTS.md/docs/devlog, branding module, empty themed window, **security baseline (fuzzed URIs/params, redacted errors, owner-only state files, no `unsafe`)** | done |
| 2 | `multi-pane-ui` | Recursive split panes, per-pane file tree + virtualized listing, selection, navigation/history, commander shortcuts (F5–F8, Tab), **dark/light/system theme from day one** (tokens only, no hard-coded colours); pane code works against any `VfsUri`, so remote backends plug in without UI rework | done, archived |
| 3 | `dnd-and-file-ops` | Cross-pane drag & drop (tree node + listing targets, Ctrl=copy/Shift=move), external file drop, `ops` pipelines, job queue with progress/cancel, conflict dialogs, trash + undo | done, archived |
| 4 | `network-backends` | **SFTP client that fills a pane** (`sftp://…` → tree + listing, connect dialog, in-memory passwords/TOFU host keys) and SMB (pavao / Windows UNC), credentials profiles, space/free-space in the status bar | next |
| 5 | `live-watch-and-packaging` | `notify`-driven directory refresh, session persistence (panes/sizes/locations), `.deb`/AppImage, `.msi`, `.dmg` | next |

**Current focus:** change 4 (remote backends) — propose with `/opsx-propose` first.

**Security is a standing concern, not a change.** The `security` OpenSpec capability starts in change 1 (hostile input, secrets, state-file permissions, symlink policy, no `unsafe`) and each change that adds a behaviour area extends it in the same change: change 3 adds drag & drop / `ops` containment and overwrite rules; change 4 adds credentials, host keys and remote listing caps. A change that adds security-relevant behaviour without a spec + failing-test proof is not done.

Out of scope until after these five: archives, previews, tabs, full-text search, plugins, CLI/MCP automation (the command registry lands in change 1 so nothing has to be re-wired later).

## Current focus

`pane-types` (terminal pane) and `pane-arrange` (drag/resize/zoom panes, Terminator-style) are implemented and awaiting real-window smoke tests before archiving.

## Ideas for future changes

Captured so they are not lost; each becomes an OpenSpec proposal (`/opsx-propose`) when picked up.

- **Preferences menu + pane-layout restore policy** (extends change 5's session persistence): a Preferences menu item holding app settings from `prefs.toml`, with *Restore previous pane layout* as `yes` / `no` / `ask`. On `ask`, a popup poses the question at startup before the saved layout is applied, and the popup repeats the same yes/no/ask control so the setting can be changed right there (the answer persists the preference). Stays a command (`prefs.*` id) and a `Prefs` field, so the security rules (no secrets, owner-only file) hold unchanged.

## Working agreement

- Nothing is "done" until `cargo test`, `cargo clippy -D warnings`, `cargo xtask layers`, `cargo xtask brand-check` and `cargo fmt --check` are green, tests are added, and `log/devlog.md` has an entry.
- Scope cuts happen in the proposal, not mid-change: if a task turns out wrong, edit the change's `tasks.md`/`design.md` before continuing.
