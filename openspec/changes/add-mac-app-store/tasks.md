## 0. Decisions before code

- [ ] 0.1 Get the user's answer on the `fm-macsec` `unsafe` exception (design §4) and on the EPL-2.0 question; record both with dates in AGENTS.md / design.md

## 1. Edition flag

- [ ] 1.1 Add the `app-store` feature to `fm-app`, `fm-engine`, `fm-ops`, `fm-ui` (forwarding); add `fm_engine::edition` with `APP_STORE` and the `EXCLUDED` command-id list
- [ ] 1.2 Filter excluded specs in `commands.rs`; test that every excluded id is unregistered under the feature and that dispatching each returns `Err`
- [ ] 1.3 Workspace load maps a terminal pane to a Files pane under the feature; test with a saved layout containing a terminal
- [ ] 1.4 Gate `engine::updates` (no startup check, no preference, no Help entry) and test no request is made
- [ ] 1.5 Gate ffmpeg discovery and show "not available in the App Store edition" in video preview; snapshot test in dark and light
- [ ] 1.6 About shows "App Store edition"; snapshot test
- [ ] 1.7 CI: macOS clippy and tests also run with `--features app-store`; Linux `cargo check --features app-store`

## 2. fm-macsec crate

- [ ] 2.1 Create `crates/macsec` (`fm-macsec`, L0), register in `xtask/src/layers.rs`, workspace `Cargo.toml`, docs crate lists; non-macOS stubs return `Unsupported`
- [ ] 2.2 Path validation (absolute, no NUL, ≤ 4 KiB) with hostile-input tests
- [ ] 2.3 Bookmarks: `create_bookmark`, `resolve_bookmark` with `ScopeGuard`; macOS-only round-trip test on a tempdir
- [ ] 2.4 `pick_folder` (non-modal `NSOpenPanel::begin`, result via channel), `workspace_open`, `workspace_eject`, `is_sandboxed`, `real_home` (`nix` user entry + container-suffix fallback, unit-tested on strings)
- [ ] 2.5 Confine any `unsafe` to `ffi.rs` with `SAFETY:` comments, or drop the module if none is needed; update `docs/architecture.md` dependency table

## 3. Engine access model

- [ ] 3.1 `FmError::AccessNotGranted { path }` with Display text and redaction-safe formatting; update exhaustive matches
- [ ] 3.2 `engine::access::Grants`: TOML store, base64 bookmarks, caps (256 × 64 KiB), atomic `0600` write; tests for corrupt, oversized, relative-path and 10 000-entry files
- [ ] 3.3 Resolve grants on the local lane at session start, keep guards, refresh stale bookmarks, mark unresolvable ones unavailable
- [ ] 3.4 `access_cmds.rs` with `access.grant` / `access.revoke` specs, param validation, `Unsupported` when not sandboxed; malformed-params fuzz coverage
- [ ] 3.5 Track dropped roots in the session; `fs-local` maps `EPERM`/`EACCES` to `AccessNotGranted` only when sandboxed and outside granted/dropped roots; tests with an injected sandbox flag
- [ ] 3.6 `user_home()` helper used by Places, Go → Home, workspace defaults and Open With; test that a container `HOME` yields the real home
- [ ] 3.7 Devices: under the sandbox without a `/Volumes` grant show the single grant entry; eject via `fm-macsec` in the App Store edition

## 4. ops and opening files

- [ ] 4.1 `ops::trash` uses `NsFileManager` when `APP_STORE`; macOS test that a file reaches the Trash
- [ ] 4.2 Open and Open With (listed apps) use `fm_macsec::workspace_open` in the App Store edition; remove spawn paths there; test that no command path spawns a process under the feature

## 5. UI and app shell

- [ ] 5.1 Listing access-needed state with Grant access… button dispatching `access.grant`; snapshot tests dark/light
- [ ] 5.2 Jobs panel action button for `AccessNotGranted` failures
- [ ] 5.3 First-run grant dialog (Home recommended, whole disk, Not now), shown once; snapshot tests
- [ ] 5.4 Preferences → Folder access list (state, Add…, Remove); Go → Grant Folder Access… menu entry
- [ ] 5.5 `fm-app` handles `Session::access_request` by calling `fm_macsec::pick_folder` and `Session::add_grant`; panes reload after a grant

## 6. Packaging

- [ ] 6.1 `packaging/macos/AppStore.entitlements.in`, `AppStore.Info.plist`, `PrivacyInfo.xcprivacy`, `review-notes.md`
- [ ] 6.2 xtask: entitlements rendering + allow-list check, plist merge, profile identifier parsing as pure functions with unit tests (run on Linux)
- [ ] 6.3 `cargo xtask mas-pkg`: universal build, `cargo packager --formats app`, merge plist, embed profile and privacy manifest, sign, `productbuild`, verification steps (design §6)
- [ ] 6.4 Bundle check fails on extra executables, missing architecture or profile/bundle-id mismatch

## 7. Release workflow

- [ ] 7.1 `macos-app-store` job in `release.yml`: secrets guard, temp keychain with both certificates (deleted in `always()`), profile and API key under `$RUNNER_TEMP`
- [ ] 7.2 `altool --validate-app`, then `--upload-package` unless `validate_only`; keep the `.pkg` as a workflow artifact
- [ ] 7.3 `workflow_dispatch`: `app-store` target and `validate_only` input; dry-run once from main and fix what it reports

## 8. Verification on a Mac

- [ ] 8.1 Install the build via TestFlight; run the sandbox smoke checklist (first run, grant Home, relaunch, ungranted folder, copy/move/trash, drop from Finder, USB stick, SMB, SFTP, network browsing, audio, PDF, Markdown) with Console filtered on `Sandbox: deny`; record results in the devlog
- [ ] 8.2 Confirm DMG build behaviour is unchanged (smoke: update check, terminal, video preview, trash Put Back)

## 9. Docs and wrap-up

- [ ] 9.1 `docs/development.md`: App Store build, secrets, `mas-pkg`, TestFlight, sandbox smoke checklist, separate settings; `docs/architecture.md`: edition gate, access model, `fm-macsec`
- [ ] 9.2 AGENTS.md §2/§3 and CLAUDE.md crate list (and the `unsafe` exception if approved)
- [ ] 9.3 CHANGELOG `[Unreleased]`: Mac App Store edition and its differences; folder-access behaviour
- [ ] 9.4 App Store Connect metadata checklist in `docs/development.md` (description, screenshots, privacy policy URL, App Privacy, category, export compliance, review notes)
- [ ] 9.5 `cargo test`/clippy for touched crates (with and without `--features app-store`), `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --all -- --check`, `cargo deny check`; append to `log/devlog.md`
