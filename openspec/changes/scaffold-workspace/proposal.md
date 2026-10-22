## Why

The repository has an approved architecture (`docs/architecture.md`) but zero code: no Cargo workspace, no crates, no guardrails, no window. Every later change (panes, drag & drop, SFTP) needs the layered workspace, the command registry, the theme-token system and the brand module to exist first — building them piecemeal inside other changes would bake in untested seams. Change 1 lays the foundation once, with its guardrails wired into CI, so changes 2–5 are feature work on a proven skeleton.

## What Changes

- New Cargo workspace (edition 2024, `unsafe_code = "forbid"`, `fm-*` crate names) with empty, compiling crates at each layer: `brand`, `vfs`, `fs-local`, `fs-smb`, `fs-sftp`, `ops`, `layout`, `engine`, `ui`, `testkit`, plus `apps/fm-app` and `xtask`.
- `cargo xtask layers` fails the build when a crate depends on a higher layer; `cargo xtask brand-check` fails it when a branding string appears outside `brand/`.
- `fm-brand` holds the single source of identity (`APP_NAME`, `APP_SLUG`, `APP_ID`, `ORG`, `EXECUTABLE`) with a working `cargo xtask rename`.
- **Security baseline lands with the skeleton, not later:** hostile-input fuzzing for `VfsUri` and command params, redaction of credentials in every error path, owner-only permissions (`0600`/`0700`) on all state files, a closed `Prefs` schema with no secret slots, and a written symlink-escape policy that `ops` tests from change 3 must satisfy. No `unsafe` anywhere (workspace forbid).
- `fm-app` opens a real window: branded title, a menu bar and status bar drawn from the command registry, dark/light/system theming (`Ctrl+Shift+T` toggles, persisted to `prefs.toml`), and a placeholder central area reading "no panes yet".
- `fm-engine` ships the command registry with a minimal seed set (`app.quit`, `view.theme.dark`, `view.theme.light`, `view.theme.system`, `view.theme.toggle`, `help.about`) and the dispatch path (`Session::dispatch(id, params)`), so menus and shortcuts never bypass it later.
- Workspace lints deny `unwrap`/`expect`/`panic`/`todo` outside tests; a smoke test proves a command with malformed params returns `Err`, not a panic.
- Dev-loop docs: `AGENTS.md` (exists) is now backed by a tree that actually compiles; `log/devlog.md` gets its first "code landed" entry.

## Capabilities

### New Capabilities

- `app-shell`: the application opens a window on Linux/macOS/Windows with the product title, a working menu bar, a status bar, and a placeholder pane area; quit closes cleanly.
- `theming`: dark, light and follow-the-system themes selected via `view.theme.*` commands (shortcut `Ctrl+Shift+T` toggles), applied to every visible surface, and persisted across restarts.
- `command-registry`: every user-visible action is a registered command (id, label, menu path, optional shortcut, `enabled`, `run`); the menu bar and shortcut routing dispatch through `Session::dispatch` only.
- `branding`: all product identity reads `fm-brand`; `cargo xtask rename` changes the visible name everywhere; `cargo xtask brand-check` fails on stale names.
- `build-guardrails`: `cargo xtask layers` enforces the L0–L5 dependency direction, workspace lints forbid panics and `unsafe`, and the CI task runs fmt/clippy/tests/layers/brand-check together.
- `security`: hostile input (URIs, command params, prefs files) cannot crash, escape or exhaust the app; secrets are never persisted, logged or shown in errors; state files are owner-only; symlinks cannot be used to write outside a destination; our code contains no `unsafe`.

### Modified Capabilities

<!-- None — this is the first change; `openspec/specs/` is empty. -->

## Impact

- **New code:** the entire `crates/*`, `apps/fm-app`, `xtask/` tree; root `Cargo.toml`, `rustfmt.toml`, `clippy.toml`.
- **Dependencies added:** `eframe`/`egui` 0.37 (`fm-app`, `fm-ui`), `thiserror`, `serde`, `crossbeam-channel`, `toml`, `directories`, and `proptest` (dev-only, for the hostile-input property suites); no network or FFI crates yet (SMB/SFTP and their `secrecy`/`zeroize` secret types land in change 4).
- **Docs:** `docs/development.md` build commands become real; `docs/roadmap.md` marks change 1 done on archive.
- **No behaviour is removed** (nothing existed); no existing specs change.
- **Risks:** first compile of winit/eframe on a bare machine (verified on this box — no extra `-dev` packages needed); keeping empty crates honest (each starts with a compiling stub API that later changes fill in rather than replace).
