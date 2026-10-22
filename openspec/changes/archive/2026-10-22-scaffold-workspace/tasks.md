# Tasks — scaffold-workspace

## 1. Workspace and guardrail tooling

- [x] 1.1 Root `Cargo.toml`: workspace (resolver 3, edition 2024, `license`, `rust-version`), members `crates/*`, `apps/*`, `xtask`; workspace `[lints]` (`unsafe_code = "forbid"`, clippy panic-family denied) and `[workspace.dependencies]` for pinned versions (eframe/egui 0.37, thiserror 2, crossbeam-channel 0.5, serde 1, toml 0.8, directories 5)
- [x] 1.2 `clippy.toml` (`allow-unwrap-in-tests = true`) and default `rustfmt.toml`
- [x] 1.3 `xtask` crate with argument parsing and the layer table `xtask/src/layers.rs` (crate → layer, allowed deps) reading real manifests
- [x] 1.4 `cargo xtask layers`: fails on higher-layer deps and unregistered crates; green on the scaffold
- [x] 1.5 `crates/brand`: `APP_NAME`/`APP_SLUG`/`APP_ID`/`ORG_NAME`/`EXECUTABLE` constants + `desktop_entry()` template; unit test that they are non-empty
- [x] 1.6 `cargo xtask rename`: rewrites brand constants, `[[bin]]` name, desktop template; records the old name in rename history; idempotent; exits non-zero with a clear message on bad input
- [x] 1.7 `cargo xtask brand-check`: stale-name scan (excludes `openspec/`, `log/`, `brand/`), `Color32::` literal scan outside `fm-ui/src/theme.rs`, menu/shortcut-vs-registry id validation; each failure names file:line
- [x] 1.8 `cargo xtask ci`: fmt check → clippy → tests → layers → brand-check, stopping at the first failure with a reproduce hint

## 2. L0–L2 crate skeletons

- [x] 2.1 `crates/vfs`: `FmError` (thiserror: NotFound, PermissionDenied, AlreadyExists, NotADirectory, Cancelled, Unsupported, Backend, InvalidUri, Io), `Scheme`, `VfsUri` (file:// parse/display/parent/join for absolute POSIX + `C:/` forms), `Entry`, the `Vfs` trait; property-style tests for uri parse/round-trip and total (non-panicking) parent/join
- [x] 2.2 `crates/fs-local`: registers `Scheme::File`, `read_dir`/`stat`… return `FmError::Unsupported` for now except `scheme()`; test asserting the scheme and the error shape
- [x] 2.3 `crates/fs-smb` and `crates/fs-sftp`: same shape, `Scheme::Smb`/`Scheme::Sftp`, no network deps yet
- [x] 2.4 `crates/testkit`: in-memory `Vfs` impl (`MemFs`) with `create_dir`/`put_file` helpers, covering every trait method so ops/engine tests need no disk
- [x] 2.5 `crates/ops`: stub types only — `ConflictPolicy`, `TransferProgress`, `JobKind`-adjacent op ids — with `FmError::Unsupported` entry points; no filesystem calls
- [x] 2.6 `crates/layout`: `PaneId`, `Axis`, `PaneNode` with `split`, `close`, `swap`, `panes()`; unit tests: ratios preserved, single-pane degenerate cases, ids stable across splits
- [x] 2.7 `cargo xtask layers` stays green after every crate lands (register each in the table as you add it)

## 3. Engine

- [x] 3.1 `crates/engine`: `CommandSpec` (id, label, menu, shortcut, params doc, `enabled`, `run`), `Registry` (insert/lookup, duplicate-id error), `Session` with `dispatch(id, params)`; unknown id → `Err`, session unchanged
- [x] 3.2 Seed commands module `app_cmds.rs` + `theme_cmds.rs` with the six commands; extend the registry list in `commands.rs`
- [x] 3.3 Graceful-failure test: every registered command fuzzed with empty/wrong-typed/oversized params returns `Ok`/`Err`, never panics
- [x] 3.4 `JobQueue`: `submit`, `poll`, `cancel`, per-lane thread configuration; a test-only command enqueues a trivial job and the test observes progress events and a terminal `Done`
- [x] 3.5 `engine::prefs`: load/save `prefs.toml` via `directories` + `toml`, `#[serde(default)]` on every field; tests for missing file, corrupt file (→ defaults, no error) and round-trip of `ThemePreference`

## 4. UI and app shell

- [x] 4.1 `crates/ui`: `Tokens` struct (semantic colours + metrics) with exhaustive `Tokens::dark()`/`Tokens::light()`; `apply_to(ctx, theme)` seeding egui visuals from `Theme::default_visuals()` and overriding only token-backed fields; test that both constructors cover every field (compile-time via destructuring)
- [x] 4.2 `apps/fm-app`: eframe (glow) window, branded title from `fm-brand`, `TopBottomPanel` status bar, central placeholder ("no panes yet")
- [x] 4.3 Menu catalogue (File/View/Help) + shortcut table in `fm-app`, dispatched through `Session::dispatch`; `Ctrl+Shift+T` wired to `view.theme.toggle`; quit persists prefs
- [x] 4.4 Startup sequence: `prefs::load` → `ctx.set_theme(pref)` → first frame; `ctx.system_theme()` respected when pref is `System`
- [x] 4.5 Headless smoke test for `Session`+menus; window launch marked `#[ignore]` with a manual check documented in `docs/development.md`

## 5. Security baseline (tests that must fail if the property regresses)

- [x] 5.1 `proptest` dev-dependency in `vfs`: property suite over `VfsUri::parse` (`..`, NUL, overlong ≥ 4 KiB components, non-UTF-8, `//`, missing scheme, 10 000-char paths) — every case `Err` or normalized-absolute `Ok`, no panic, bounded allocation
- [x] 5.2 `VfsUri::parent()`/`join()` properties: never yields `..`, never loses absoluteness, total on hostile input
- [x] 5.3 `dispatch` input caps: reject ≥ 1 MiB params and nesting depth > 32 *before* the command runs, with a test for each cap; oversized-param fuzz over all registered commands stays panic-free
- [x] 5.4 `FmError::Display` redaction: unit test walks every variant with `user:password@host`- and `password=`-shaped content and asserts no credential leaks; add a `redact()` helper with its own property test (never panics, output has no `:secret@`)
- [x] 5.5 `prefs::save` on Unix creates mode `0600` file / `0700` dir (temp config dir + `metadata().permissions().mode()`); write via temp file + atomic rename (test: existing prefs preserved when the temp write fails)
- [x] 5.6 Hostile `prefs.toml` cases: invalid TOML, 10 MiB+ file, absurd values, billion fields → defaults, bounded, no panic, no truncate/append side effects
- [x] 5.7 Closed `Prefs` schema test: serialize a `Prefs` and assert the JSON/ TOML key set is exactly the allowed non-secret fields (catches anyone adding a `password` field later)
- [x] 5.8 Symlink policy: document the rule in `ops` (`SymlinkPolicy` type + doc tests in the stub) so change 3 implements against an agreed contract; no filesystem calls yet

## 6. Gate, docs, wrap-up

- [x] 6.1 Full gate green: `cargo test --workspace` (includes every security test above), `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all -- --check`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo xtask ci`
- [x] 6.2 Prove the guardrails actually fail: temporarily add a bad dep, a hard-coded name, a `Color32::` literal, an `unwrap()` in non-test code, a credential-shaped string in an error, and a `0644` prefs file — each must be caught — then revert
- [x] 6.3 `docs/development.md`: real build/run commands, the `#[ignore]` window checklist, config-dir paths; `docs/architecture.md` §13 threat model kept in sync with what landed
- [x] 6.4 Append `log/devlog.md` entry (what landed, test counts, still open) and mark change 1 done in `docs/roadmap.md`
- [x] 6.5 Commit the scaffold when the user asks; then `/opsx-archive scaffold-workspace`
