## Context

Today macOS ships one way: `cargo-packager` builds a Developer ID–signed, hardened,
notarized arm64 DMG (`release.yml`, `xtask/src/brand.rs` writes `Packager.toml`).
The Apple side of a store build already exists (checked 2026-11-23):

- Provisioning profile `FileManagerUltra MAS`, platform OSX, application identifier
  `<team>.com.filemanagerultra.mac` (matches `fm_brand::MACOS_BUNDLE_ID`), Apple
  Distribution certificate, expires 2027-11-15.
- Repository secrets: `DIST_CERT_P12/_PASSWORD` (Apple Distribution),
  `INSTALLER_CERT_P12/_PASSWORD` (Mac Installer Distribution), `MAS_PROFILE`,
  `ASC_KEY_P8`, `ASC_KEY_ID`, `ASC_ISSUER_ID`, `APPLE_TEAM_ID`, plus the existing
  `DEVID_*`. The local copies live in the gitignored `macsigning/`.

The checklist in `macsigning/instructions.txt` assumes an Xcode project
(`xcodebuild archive` / `-exportArchive`). There is none: the app is a Cargo binary.
The design reaches the same result (signed `.app` in a signed `.pkg`, uploaded with
the API key) with `codesign`, `productbuild` and `altool`.

Things in the current code that a sandbox breaks: `directories` returns the container
as home; the local backend reads `/` and `/Volumes`; ffmpeg, `diskutil`, `open`, the
terminal shell and custom Open With commands are spawned processes; `trash` 5 on macOS
defaults to scripting Finder; `engine::updates` calls GitHub and tells users to
download a DMG.

## Goals / Non-Goals

**Goals:** a store build that passes App Review on the first serious attempt; access
that feels like one confirmation, not constant prompts; a release job that uploads
without manual signing; the DMG build stays exactly as capable as today.

**Non-Goals:** sandboxing the DMG build; bundling ffmpeg or a shell; in-app purchases
or a paid/lite split; iCloud or file-provider integration; automatic submission for
review; macOS drag-out (still unimplemented in both builds).

## Decisions

### 1. Compile-time edition, not a runtime switch
A cargo feature `app-store` on `fm-app` forwards to `fm-engine/app-store`,
`fm-ops/app-store` and `fm-ui/app-store`. `fm_engine::edition::APP_STORE` is a `const
bool` (`cfg!(all(feature = "app-store", target_os = "macos"))`), so off-macOS the
feature is inert and CI on Linux can still type-check it.
Sandbox detection at runtime (`APP_SANDBOX_CONTAINER_ID` env) is used only for the
access model (§3), so a locally sandboxed dev build also behaves correctly.
*Alternative:* a runtime flag read from `Info.plist`. Rejected: unsupported code would
still ship in the store binary, and Apple's review scans for update mechanisms.

### 2. Unsupported features are unregistered, not disabled
`commands.rs` aggregates `specs()`; under `APP_STORE` the update, terminal, run and
ffmpeg-only specs are filtered out by id list (`edition::EXCLUDED`). Menus, palette
and shortcut tables are already built from the registry, so they follow. A test
asserts every excluded id is absent and that each of them dispatched returns
`Err`. Workspace loading maps an unknown pane kind (terminal) to a Files pane.
Preview code checks `edition::APP_STORE` before ffmpeg discovery and shows the
"not available in the App Store edition" text.
*Alternative:* register them with `enabled = false`. Rejected: greyed-out items that
can never work invite review questions and user confusion.

### 3. Access model: security-scoped bookmarks owned by the engine
- **`fm-macsec` (L0, new)**: thin, safe API over Foundation/AppKit via `objc2` 0.6 /
  `objc2-foundation` / `objc2-app-kit` 0.3 (already in `Cargo.lock` via winit, same
  licences). Functions: `create_bookmark(path) -> Vec<u8>`,
  `resolve_bookmark(&[u8]) -> Resolved { path, stale, scope: ScopeGuard }`
  (`ScopeGuard` calls `stopAccessingSecurityScopedResource` on drop),
  `pick_folder(start, message, callback)` (non-modal `NSOpenPanel::begin` with a
  completion block, run on the main thread, result sent over a channel),
  `workspace_open(path, app: Option<path>)`, `workspace_eject(path)`, `real_home()`,
  `is_sandboxed()`. Non-macOS: same API, every call returns `Unsupported`.
  All paths are validated (absolute, no NUL, ≤ 4 KiB) before any OS call.
- **`engine::access`**: `Grants` store (`grants.toml` in the state dir, base64 bookmark
  per entry, caps 256 × 64 KiB, atomic `0600` write). At session start a job on the
  `local` lane resolves all grants and holds the guards in the session; because the
  local lane is FIFO, it runs before the first listing job. Stale bookmarks are
  recreated from the resolved URL.
- **Commands** (`access_cmds.rs`): `access.grant { path? }` sets
  `Session::access_request`; `fm-app` sees it on the next frame, calls
  `fm_macsec::pick_folder`, and on confirm calls `Session::add_grant(path)`, which
  creates the bookmark on the local lane and refreshes affected panes.
  `access.revoke { path }` drops the guard, removes the entry and refreshes.
  `fm-app` owning the panel call matches how it already owns window-level platform
  calls (drag-out); no filesystem I/O happens there.
- **Errors**: `FmError::AccessNotGranted { path }`. `fs-local` maps `EPERM`/`EACCES`
  to it only when `is_sandboxed()` and the path is not under a granted or dropped
  root; otherwise the existing permission error stays. The pane's listing view shows
  the explanation + Grant button for this variant; the jobs panel message carries an
  action button dispatching `access.grant`.
- **Dropped items**: drop handling records dropped roots in the session (not
  persisted) so the mapping above does not misreport them.
- **Real home**: `nix::unistd::User::from_uid(getuid())` (safe, `nix` is already in the
  lock) → `dir`; fallback strips `Library/Containers/<MACOS_BUNDLE_ID>/Data` from
  `HOME`. `places_cmds`, `goto_cmds::home_dir`, `workspace` defaults and
  `openwith_cmds` use one engine helper `user_home()` instead of `directories`'
  `home_dir()`. `prefs.rs` keeps `directories::ProjectDirs` (the container is the
  right place for app state).

*Alternatives:* temporary-exception entitlements for `/` (rejected by review for new
apps); asking per folder without a first-run dialog (constant prompts); the
`com.apple.security.files.all` key (does not exist for third parties).

### 4. `unsafe` in `fm-macsec`
`objc2-foundation` 0.3 marks most of these methods safe, but some (block-based
completion handlers, `bookmarkDataWithOptions…` error out-params) may be `unsafe`.
The crate is set up like `fm-winsec`: `unsafe_code = "deny"` in its own lints, and
`#[allow(unsafe_code)]` only on `ffi.rs`, every block `SAFETY:`-commented. If the
implementation finds every needed call safe, `ffi.rs` is not created and the crate
uses workspace lints. **This needs the user's approval before implementation**; it
is recorded in AGENTS.md §3 rule 2 when approved.

### 5. Trash
`ops::trash` uses `trash::TrashContext` with
`DeleteMethod::NsFileManager` (via `trash::macos::TrashContextExtMacos`) when
`APP_STORE`, else the current default. No Apple Events entitlement needed.

### 6. Packaging: `cargo xtask mas-pkg`
Runs on macOS only:
1. Build `fm-app --features app-store` for `aarch64-apple-darwin` and
   `x86_64-apple-darwin`, `lipo -create` into one binary.
2. `cargo packager --formats app` with a generated `Packager.toml` (no signing
   identity) to get the bundle layout, icons and base `Info.plist`.
3. Merge `packaging/macos/AppStore.Info.plist` keys (category, encryption flag,
   local-network usage string, `NSBonjourServices` from `fm-discovery`'s service
   list, `LSMinimumSystemVersion` same as DMG), copy `PrivacyInfo.xcprivacy` into
   `Contents/Resources`, copy the profile to `Contents/embedded.provisionprofile`.
4. Render `packaging/macos/AppStore.entitlements.in` with `APPLE_TEAM_ID`, check it
   against the allow-list (the same function the unit test uses), then
   `codesign --force --options runtime --timestamp --entitlements … --sign "Apple Distribution"`.
5. `productbuild --component <app> /Applications --sign "3rd Party Mac Developer Installer" <pkg>`
   (the Mac Installer Distribution certificate's identity name).
6. Verify: `lipo -archs`, `codesign --verify --strict`, entitlements dump equals the
   allow-list, `security cms -D` of the embedded profile matches bundle id and team,
   `pkgutil --check-signature`.
The pure parts (entitlements rendering/allow-list, plist merge, profile identifier
parsing) are plain Rust with unit tests that run on Linux.

### 7. Release job
New matrix-independent job `macos-app-store` in `release.yml` on `macos-15`: guard on
secrets (skip with notice), temp keychain from `DIST_CERT_P12` + `INSTALLER_CERT_P12`
(deleted in an `always()` step), profile and `AuthKey_<id>.p8` written under
`$RUNNER_TEMP` (`API_PRIVATE_KEYS_DIR` points there), `cargo xtask mas-pkg`,
`xcrun altool --validate-app` then `--upload-package` with `--apiKey/--apiIssuer`.
`workflow_dispatch` gains `app-store` in the target options plus a `validate_only`
boolean. The `.pkg` is kept as a workflow artifact (not attached to the GitHub
release — it is useless outside the store).
*Alternative:* `iTMSTransporter` or fastlane. `altool` ships with Xcode and supports
API keys; no extra tooling.

### 8. Privacy manifest and review metadata
`PrivacyInfo.xcprivacy`: `NSPrivacyTracking = false`, no collected data types,
accessed API reasons: file timestamps `3B52.1` (files the user grants), disk space
`E174.1` (show free space / check before copy), system boot time `35F9.1` (`Instant`
→ `mach_absolute_time`), user defaults `CA92.1` (winit/AppKit). Review notes
(`packaging/macos/review-notes.md`, pasted into App Store Connect) justify each
entitlement: user-selected + bookmarks (file manager, persistent folder grants),
network.client (SMB/SFTP connections the user opens), network.server (mDNS /
WS-Discovery replies for network browsing). Export compliance: only standard
protocols (SSH, SMB3, TLS) via standard libraries → exempt → `false`.

## Risks / Trade-offs

- [App Review rejects network.server] → discovery is the only user; if rejected, drop
  the key and hide network discovery in the App Store edition (users can still type
  `smb://` / `sftp://` addresses).
- [Review objects to a file manager that needs a whole-disk grant] → the first-run
  dialog recommends Home; whole disk is an explicit user choice in the system panel,
  as other store file managers do.
- [Sandbox denials we did not anticipate (e.g. `statfs` on volumes, `notify`/FSEvents
  on granted folders)] → manual sandbox smoke checklist in `docs/development.md`
  (Console filtered on `Sandbox: deny`), run on a TestFlight build before the first
  submission.
- [No Mac in the loop for every change] → CI runs clippy/tests with `--features
  app-store` on macOS; `mas-pkg` dry-run via `workflow_dispatch` validates signing
  without uploading.
- [Universal build doubles macOS CI time] → only the App Store job builds x86_64;
  the DMG stays arm64.
- [EPL-2.0 (`merman-elk-layered`) in a store binary] → EPL-2.0 permits distribution of
  object code under other terms with source available; the notice is already shown in
  About. Flag for the user to confirm before the first submission.
- [Store users miss video/terminal] → the store description and About say the DMG
  from the website has the full feature set.
- [Grants file corruption locks users out] → corrupt entries are skipped and
  re-granting is one click from the access-needed state.

## Migration Plan

No migration: the App Store edition starts with an empty container. Rollback =
remove the build from sale in App Store Connect; the DMG is unaffected. First
submission: upload via the release job, test via TestFlight, fill App Store Connect
metadata (description, screenshots, privacy policy URL, App Privacy "no data
collected", category Utilities, price), paste review notes, submit manually.

## Open Questions

1. Approve the confined `unsafe` exception for `fm-macsec` (only if `objc2` requires
   it; decided during task 2).
2. Price and availability (free like the DMG, or paid?) — App Store Connect only, no
   code impact.
3. Should the terminal come back later, restricted to granted folders? Out of scope
   here; revisit after the first review.
4. Confirm the EPL-2.0 dependency is acceptable in a store binary.
