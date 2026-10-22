# Design — scaffold-workspace

## Context

`docs/architecture.md` defines the product: a layered Rust workspace (L0 `vfs`/backends … L5 `fm-app`), everything user-visible a command, theming by semantic tokens, brand identity in one crate. None of it exists as code yet; `openspec/specs/` is empty. This change creates the skeleton that changes 2–5 build on, with the guardrails (layers, brand, panic-free lints) wired from the first commit so they never have to be retrofitted. Verified on the target machine: `eframe 0.37` compiles on stock Ubuntu 24.04 with no extra `-dev` packages; Rust `1.99.1` is installed.

## Goals / Non-Goals

**Goals**

- A workspace that compiles and tests green on Linux today, with Windows/macOS kept buildable.
- Real guardrails: `xtask layers`, `xtask brand-check`, `xtask rename`, `xtask ci`, panic-free lints.
- A window that opens, is branded and themed (dark/light/system), persists prefs, and dispatches its menus through a command registry.
- Typed stub APIs in the non-UI crates so later changes fill in behaviour instead of reshaping signatures.

**Non-Goals**

- Panes, trees, listings, drag & drop (change 2/3), file operations, network backends (change 4), watchers (change 5).
- Windows/macOS release packaging, installers, code signing.
- CLI, control channel, MCP.
- Session (pane layout) persistence — only `prefs.toml` here.

## Decisions

**D1 — Typed stub crates, not empty folders.** Each crate ships its public surface with minimal logic: `vfs` gets `FmError`, `Scheme`, `VfsUri`, `Entry` and the `Vfs` trait (methods returning `FmError::Unsupported` where no backend exists); `engine` gets `Session`, `Registry`, `CommandSpec`, `dispatch`, and a minimal `JobQueue`; `ui` gets `Tokens` (`dark()`/`light()`) and theme application; `layout` gets `PaneId`/`PaneNode` with `split`/`close`/`swap` invariants tested. *Why:* the layering check, imports and names are then real, and change 2 edits rather than invents. *Alternative rejected:* empty `lib.rs` files — they defer every naming decision and make `xtask layers` a no-op.

**D2 — Six seed commands only.** `app.quit`, `app.about`, `view.theme.dark|light|system|toggle`, with `enabled: |_| true`. Menu catalogue and shortcut table live in `fm-app` as data, validated against the registry by `brand-check`. *Why:* proves the "everything is a command" spine without inventing behaviours this change doesn't have.

**D3 — Theming on egui's own types.** `Session::prefs.theme: egui::ThemePreference` (`System` default); `fm-app` calls `ctx.set_theme(pref)` at startup and after each `view.theme.*` command. `Tokens` is a plain struct of semantic colours + metrics; `dark()` and `light()` destructure it exhaustively so a new field breaks the build until both palettes define it. egui visuals are seeded from `Theme::default_visuals()` then overridden only where tokens differ.

**D4 — Prefs in the engine, TOML on disk.** `engine::prefs::{load, save}` using `directories` for the config dir (`$XDG_CONFIG_HOME/<slug>` etc.), `toml` + `serde`. Missing/corrupt file → defaults, silently (never a user-facing error, never a panic). Written on change and on quit. *Why:* the engine is L3 and may own I/O; the UI stays free of filesystem calls.

**D5 — Minimal job queue now.** `JobQueue` with `crossbeam-channel`, one thread per lane (`local` 4 / `smb` 1 / `sftp` 2 as configuration, only `local` used in tests here), `submit(kind, …) -> JobHandle`, `poll() -> Vec<JobEvent>`, `cancel`. Proved by a test-only command. *Why:* command-registry spec requires long actions to be jobs; a stub now keeps the dispatch contract honest without building `ops`.

**D6 — Renderer: `eframe` with `glow`.** A file manager has no GPU workload; glow is lighter and keeps cold builds shorter. Upgrade path to `wgpu` is a one-line feature change in `fm-app`.

**D7 — `xtask` is one binary, four subcommands** (`layers`, `brand-check`, `rename`, `ci`), reading a table `xtask/src/layers.rs` that maps crate → layer and crate → allowed deps. `brand-check` also runs the colour-literal scan (theming spec) and the menu/shortcut-vs-registry validation (command-registry spec). *Why:* one tool, no external binaries, the same entry point CI and agents use. *Alternative rejected:* separate scripts per check — they drift.

**D8 — Rename is table-driven.** `cargo xtask rename "NewName"` rewrites `brand/src/lib.rs` constants, the `[[bin]]` name in `apps/fm-app/Cargo.toml`, and the desktop-entry template; it records the old name in a `rename-history` list inside `xtask` so `brand-check` keeps scanning for it. *Why:* "renamable in one commit" is a hard requirement (branding spec).

**D9 — Lint configuration.** Root `clippy.toml` with `allow-unwrap-in-tests = true`; workspace `[lints]` denies the panic family in non-test code and forbids `unsafe`. `rustfmt.toml` stays default.

**D10 — Pinned versions.** `eframe`/`egui` `0.37`, `thiserror` `2`, `crossbeam-channel` `0.5`, `serde` `1`, `toml` `0.8`, `directories` `5`. No FFI, no tokio, no network crates in this change — they arrive with change 4 behind the `Vfs` trait.

**D11 — Security is scaffold work, not a later pass.** Three concrete mechanisms land now:

- *Hostile-input testing tooling.* `proptest` as a dev-dependency everywhere attacker-controlled parsing exists. `VfsUri::parse` gets a property suite (`..`, NUL, overlong, non-UTF-8, `//`, huge paths → `Err` or a normalized absolute result, bounded allocation); the command registry gets a **cap on parameter sizes** (reject ≥ 1 MiB strings, nesting depth > 32) enforced in `dispatch` *before* the command runs, plus the per-command fuzz suite. *Why:* the panic-free rule is only as good as the tests that prove it.
- *Secrets policy.* `Prefs` is a closed struct (theme + window state) — there is no field that *could* hold a password, so nothing can accidentally persist one. `FmError::Display` (and every `Backend { message }` construction) runs through a redactor that strips `user:password@` and `password=`-shaped substrings; a test walks every error variant with credential-shaped input. When change 4 adds connections, secrets use the `secrecy`/`zeroize` stack (in-memory, zeroized on drop, never `Debug`-printed) and the `security` spec grows matching requirements.
- *State-file hardening.* `prefs::save` creates the file with mode `0600` and the config dir `0700` on Unix (`OpenOptionsExt`; Windows gets a restrictive ACL via the same code path guarded by `cfg`), writes through a temp file + rename so a crash cannot leave a half-written prefs file, and caps the read size (10 MiB) before parsing.

**D12 — Symlink policy is fixed now, tested in change 3.** Rule: a write operation never follows a symlink that resolves outside the destination root it was given; sources that are symlinks are copied as links (or dereferenced only within the same root), never silently across roots. `ops` owns enforcement (`realpath`-style resolution with a bounded walk); this change records the rule in the `security` spec and ships the helper types in `ops`, so change 3 writes tests against an already-agreed contract instead of inventing policy mid-implementation.

**D13 — Threat model written down.** `docs/architecture.md` §13 names the three attacker classes we defend against from day one: hostile *files/names* on any volume, hostile *remote servers* (SMB/SFTP listings, error strings), and hostile *local input* (drop payloads, command params, prefs files). Each requirement in the `security` spec maps to one of them, so "is this covered?" has an answer.

## Risks / Trade-offs

- **Stub APIs freeze too early** → they are deliberately thin (types + trait signatures); change 2 owns pane behaviour and may extend, never the scaffold's job to guess. Review each stub against the change that consumes it.
- **`xtask layers` table drifts from `Cargo.toml`** → `layers` reads the real manifests and fails on unknown crates, so drift is an error, not silence.
- **`brand-check` grep false positives** (the name inside archived specs or this change's own files) → the scan excludes `openspec/`, `log/` and `brand/` by rule, and matches whole words.
- **Headless/CI cannot open a window** → the shell's logic lives behind `Session` + registry, which are testable headlessly; the window itself gets one smoke test behind `#[ignore]` and a manual checklist item in `docs/development.md`.
- **First `cargo build` is slow (~2–4 min cold)** → acceptable; `target/` is shared and incremental afterwards. Parallel agents use per-agent `CARGO_TARGET_DIR`.
- **A green fuzz suite creates false confidence** → the property suites are small and targeted (they prove *no panic and bounded allocation*, not correctness); the `security` spec explicitly keeps change 3/4 from re-using the same tests as a substitute for real credential/host-key handling.
- **Redaction by regex is leaky** → the redactor is a last line of defence; the primary rule is "credentials never enter error types at all" (typed `Secret` wrapper in change 4, never `Display`ed). The test asserts both the wrapper and the redactor.
- **Permissions differ per platform** → Unix modes are asserted in CI (Linux); the Windows ACL path compiles under `cfg` and is verified manually on a Windows box in change 5 (packaging), with the test `#[cfg(unix)]`-gated and a documented checklist item.

## Migration Plan

Nothing to migrate (first code). Rollback is `git revert` of the scaffold commit; no data or user settings exist in the wild yet. `prefs.toml` schema is introduced here and is forward-compatible (`serde(default)` on new fields from day one).

## Open Questions

- **Window class / app id on Linux**: use `APP_SLUG` for `StartupWMClass` now; revisit when change 5 adds the `.desktop` file (X11 vs Wayland quirks).
- **macOS menu bar**: egui renders the menu inside the window; on macOS the native app menu may need `eframe`'s viewport menu integration — flagged for change 2 when more commands exist.
- **MSRV**: not pinned in this change; pin once CI runs on more than one toolchain.
