## Why

The gate (`cargo xtask ci`) only ran when someone remembered to run it: no workflow checked pushes or pull requests (Dependabot PRs included), and nothing stopped an unformatted or clippy-failing commit locally. The panic bans also left two crash classes open on hostile input — unchecked indexing (`v[i]`, `&b[a..b]`) and byte slicing of `str` that can split a UTF-8 character — and dependency licences and advisories were checked by hand.

## What Changes

- New `ci` GitHub workflow on every push to `main` and every pull request: format, clippy with warnings denied (Linux, plus clippy on Windows and macOS for platform-only code), layers, branding, dependency policy, unused dependencies, tests and doctests; pixel snapshots and a mess report are informational.
- Workspace lints deny `clippy::indexing_slicing` and `clippy::string_slice` in non-test code and warn on `unused_qualifications` and `clippy::redundant_clone` (warnings are errors in the gate). All existing sites are rewritten with `get()`, slice patterns, `split_once`/`strip_prefix` or iterators.
- `deny.toml`: permissive licences only, the approved EPL-2.0 exception scoped to `merman-elk-layered`, RustSec advisories and yanked crates fail (ignores need a reason), crates.io only.
- Versioned git hooks in `.githooks/`, installed with `cargo xtask hooks`: pre-commit runs format, layers and branding; pre-push runs workspace clippy and the dependency checks.
- `cargo xtask ci` adds the dependency policy and unused-dependency steps and uses the same clippy flags as CI (`--all-features --locked`).
- `cargo xtask mess`: report-only list of the longest and most complex functions.

## Capabilities

### Modified Capabilities
- `build-guardrails`: indexing/slicing lints, the dependency policy, CI on pushes and PRs, git hooks and the mess report.

## Impact

- New files: `.github/workflows/ci.yml`, `deny.toml`, `.githooks/pre-commit`, `.githooks/pre-push`, `xtask/src/hooks.rs`, `xtask/src/mess.rs`.
- Root `Cargo.toml` lints; ~200 call sites across most crates rewritten to bounds-checked access (no behaviour change); `Cargo.lock` moves two yanked crates (`clipboard-win`, `crypto-bigint`) to current patch releases.
- Docs: `AGENTS.md` §4, `CLAUDE.md`, `docs/development.md`, `docs/contributing.md`, `.github/dependabot.yml`.
- Contributors need `cargo-deny` and `cargo-machete` for `cargo xtask ci`; the hooks skip them with a note when absent.
