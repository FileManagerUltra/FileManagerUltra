## 1. Lints

- [x] 1.1 Deny `clippy::indexing_slicing` and `clippy::string_slice`, warn `unused_qualifications` and `clippy::redundant_clone` in `[workspace.lints]`
- [x] 1.2 Apply the machine-applicable fixes (`cargo clippy --fix`) for qualifications and redundant clones
- [x] 1.3 Rewrite every indexing/slicing site in non-test code and test helpers to bounds-checked access
- [x] 1.4 Clippy clean on Linux and for the Windows-only crates (`x86_64-pc-windows-gnu`)

## 2. Dependency policy

- [x] 2.1 `deny.toml`: licence allow-list, scoped EPL-2.0 exception, advisories with reasoned ignores, crates.io only
- [x] 2.2 Update the yanked `clipboard-win` and `crypto-bigint`; replace the deprecated `bits()` with `bit_width()`
- [x] 2.3 `cargo machete` clean outside `vendor/`

## 3. Gate, hooks and CI

- [x] 3.1 `cargo xtask ci`: shared clippy flags, `cargo deny` and `cargo machete` steps, early check for missing tools
- [x] 3.2 `cargo xtask hooks [install|uninstall]`, `cargo xtask hook <name>`, `.githooks/` scripts, tests
- [x] 3.3 `cargo xtask mess` report with parser tests
- [x] 3.4 `.github/workflows/ci.yml` (lint, test, clippy on Windows/macOS) using only allow-listed actions

## 4. Docs

- [x] 4.1 `AGENTS.md`, `CLAUDE.md`, `docs/development.md`, `docs/contributing.md`, `docs/architecture.md`, `.github/dependabot.yml`
