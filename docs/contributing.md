# Contributing

## Style

UI dialogs follow the [dialog style guide](style-guide.md), with shared chrome and
typography in `fm-ui::dialog_style`.

- `cargo fmt` (default settings) is law; `cargo clippy --all-targets -- -D warnings` must be clean.
- No `unsafe`, no `unwrap`/`expect`/`panic`/`todo!` outside `#[cfg(test)]` — see `AGENTS.md` golden rules.
- Errors: one workspace error type per layer (`FmError`), always with context (typed variants, not string soup); its `Display` redacts credentials.
- Comments explain *why*, not *what*. Keep public APIs documented with `///`; private helpers do not need prose.
- Names: crates/dirs use `fm-*`; commands use dotted ids (`file.copy`, `pane.split.right`, `nav.back`).
- Colours: only via `fm-ui/src/theme.rs` semantic tokens; `Color32::` literals are allowed there alone. New token ⇒ define it in **both** `Tokens::dark()` and `Tokens::light()`, and eyeball the widget in both themes (snapshots run twice).

## The "add a command" checklist

1. New module `crates/engine/src/<area>_cmds.rs` exposing `specs() -> Vec<CommandSpec>`; register it in `crates/engine/src/commands.rs`.
2. Implement `run` against `Session` only — no direct filesystem calls, no egui.
3. Long work enqueues a `Job` and returns `Ok(())` immediately.
4. Wire menu + shortcut in `apps/fm-app` (the id must match the registry exactly).
5. Tests: happy path, `enabled` false cases, malformed params → `Err`.
6. Run the §4 gate from `AGENTS.md` and note it in `log/devlog.md`.

## Changes flow (OpenSpec)

1. `/opsx-propose "what and why"` → creates `openspec/changes/<name>/` with `proposal.md`, `specs/`, `design.md`, `tasks.md`.
2. Review the proposal; implementation starts only after it exists.
3. `/opsx-apply` works through `tasks.md`; keep the tree green.
4. `/opsx-archive` moves the change to `openspec/changes/archive/` and merges its deltas into `openspec/specs/`.
5. Structural changes update `docs/architecture.md` in the same change.

## Commits

- Small, single-purpose; subject in imperative mood (`add virtualized listing`, not `added`).
- Never commit `target/`, `log/`, corpora or secrets.
- A commit should leave the workspace compiling and the tests you touched green.
