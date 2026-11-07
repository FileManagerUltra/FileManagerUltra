## 1. Crate and shared logic

- [x] 1.1 Create `crates/winout` (`fm-winout`, L0) with its own lints (`unsafe_code = "deny"`, clippy bans), register in `xtask/src/layers.rs`, root `Cargo.toml` and `apps/fm-app/Cargo.toml`; non-Windows build exposes the same API returning `Unsupported`
- [x] 1.2 Safe module: validate the path list (absolute, no NUL, count cap, reuse `fm_dragout` limits) and encode the wide-char path list; unit tests for empty/relative/NUL/over-cap/long paths and exact encoding (run on every OS; verbatim-prefix stripping)

## 2. OLE drag source (Windows only)

- [x] 2.1 `ole.rs` (the only module allowing `unsafe`): shell data object (`SHCreateDataObject`), `IDropSource` with Escape/button handling, `OleInitialize` on a helper thread, `DoDragDrop` with Copy|Move, result via channel; document every `unsafe` invariant
- [x] 2.2 Public API `drag(paths)` (blocking, UI thread) returning `Outcome::{Dropped, NoTarget, Cancelled}` or `DragError`; never panics, HRESULT failures map to `DragError::System`

## 3. App wiring

- [x] 3.1 `fm-app/src/dragout.rs`: start the Windows drag when a payload leaves the window; reuse the button-release injection and status notices; macOS keeps the "not supported" notice
- [x] 3.2 Verify local-only payload rule and notice text are shared with Linux paths (remote items refused)

## 4. Verification and docs

- [x] 4.1 (fm-winout checked and clippy-clean for windows-gnu; fm-app not compiled for Windows locally because `ring` needs mingw gcc, so CI/Windows build is its first check) `cargo check --target x86_64-pc-windows-gnu` (and msvc in CI) for `fm-winout` and `fm-app`; Linux tests, clippy `-D warnings`, `cargo xtask layers`, `brand-check`, fmt
- [ ] 4.2 Manual Windows smoke check in `docs/development.md`: drop into Explorer, Chrome/Edge upload field, Outlook/Teams; Escape cancel; release over nothing; click works afterwards; decide helper-thread vs UI-thread per design question
- [x] 4.3 Docs: `docs/architecture.md` (drag out + unsafe exception), `AGENTS.md` golden rule 2, `docs/roadmap.md` (remove Windows from open items), `CHANGELOG.md` Added line, devlog entry
- [ ] 4.4 Sync specs and archive the change after the Windows smoke check passes
