# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

@AGENTS.md

`AGENTS.md` (imported above) is the primary guide: golden rules, layering, the
end-of-task checklist, versioning and release steps. This file only adds what
helps when working in Claude Code.

## Commands

```sh
cargo run -p fm-app                                  # run the app (needs a display)
cargo test -p <crate> --lib --tests                  # fast inner loop, skips doctests/examples
cargo test -p fm-engine <test_name_substring>        # single test (or module path, e.g. commands::tests::fuzz)
cargo test -p fm-engine -- --exact <path::to::test>  # exactly one test
cargo test -p fm-ui --test snapshots                 # egui_kittest snapshots (skip without a GPU/CPU adapter)
UPDATE_SNAPSHOTS=1 cargo test -p fm-ui --test snapshots   # rewrite committed PNGs
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo xtask ci                                       # everything CI runs
```

- Crate package names are `fm-<dir>` (`crates/fs-local` → `fm-fs-local`, `apps/fm-app` → `fm-app`).
- Linux builds need `pkg-config` + `libasound2-dev` (audio via `cpal`).
- On memory-constrained machines: `CARGO_BUILD_JOBS=1` and `-- --test-threads=1`; avoid parallel builds (an OOM can kill the whole desktop scope).
- `fm-fs-smb` integration tests use Docker (`dperson/samba`) and skip themselves when unavailable.
- Never delete `crates/ui/tests/snapshots/` — they are committed baselines.

## Crates not listed in AGENTS.md §2

The authoritative layer table is `xtask/src/layers.rs`. Besides the crates in
AGENTS.md: L0 `fm-discovery` (mDNS/WS-Discovery network hosts), `fm-git`
(read-only `gix` status/diff), `fm-dragout` (X11 XDND drag source),
`fm-wayland`, `fm-winout`; L1 `fm-term` (embedded terminal), `fm-preview`
(Markdown/text/SVG/log preview models).

## Big-picture flow

- **Command dispatch:** UI widgets in `fm-ui` and shortcuts/menus in `fm-app`
  only dispatch command ids. Commands live in `crates/engine/src/*_cmds.rs`, each
  exposing `specs()`, aggregated in `engine/src/commands.rs` (which also holds the
  malformed-params fuzz tests every command must pass).
- **Session/frame loop:** the UI thread reads `Session` state and calls
  `Session::poll()` once per frame; workers on per-scheme lanes (`local`, `smb`,
  `sftp`) stage results into an inbox that only `poll()` applies (e.g. to
  `DirCache`). File jobs (`engine::fileops`) report throttled progress and park on
  a decision channel for conflicts. Long walks (properties sizes, Find Files,
  Open With scan), thumbnails and Markdown images use their own bounded pools,
  not job lanes. Details: `docs/architecture.md` §6–7.
- **Filesystems:** every backend implements the `Vfs` trait over `VfsUri`
  (`crates/vfs`); features must work identically on local, SMB and SFTP. Use
  `fm-testkit`'s in-memory `Vfs` (and `hostile_tree()`) for side-effect tests.
- **Errors:** dispatch failures set `Session::notice`, surfaced in the jobs panel
  message log; short confirmations use `Session::info`.

## OpenSpec workflow

Specs live in `openspec/specs/`, in-flight changes in `openspec/changes/<name>/`
(`proposal.md`, `design.md`, `tasks.md`, `specs/`). The `openspec` CLI is
installed. The propose/apply/archive/explore/sync workflows exist twice, with
the same content: for GitHub Copilot as `.github/prompts/opsx-*.prompt.md` +
`.github/skills/openspec-*/SKILL.md` (invoked as `/opsx:apply`), and for
OpenCode as `.opencode/commands/` + `.opencode/skills/` (`/opsx-apply`). In
Claude Code, read the matching `SKILL.md` and follow its steps (propose → apply,
ticking `tasks.md` → archive, syncing deltas into `openspec/specs/`). If you edit
one copy, keep the other in sync.

`log/devlog.md` is gitignored and local; still append to it at the end of a task.
