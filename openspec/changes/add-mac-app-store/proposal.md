## Why

The notarized DMG reaches only users who find the website; the Mac App Store adds
discovery, trusted installation and automatic updates. Apple accepts only sandboxed
apps, and a sandboxed file manager cannot read the disk until the user grants
folders, so the store build needs a designed access model, not a re-signed DMG.
The Apple side is ready: the Apple Distribution and Mac Installer Distribution
certificates, the `FileManagerUltra MAS` provisioning profile for
`com.filemanagerultra.mac` (valid until 2027-11-15) and the App Store Connect API key
are stored as repository secrets.

## What Changes

- An **App Store edition**: the same app built with a cargo feature (`app-store`),
  sandboxed, signed with Apple Distribution plus the embedded provisioning profile,
  packaged as a signed `.pkg` and uploaded to App Store Connect by the release
  workflow. The website DMG stays the full, unsandboxed Developer ID build.
- **Folder access grants**: a first-run "Grant access" step (home folder recommended,
  whole disk possible), security-scoped bookmarks that survive relaunch, a "Grant
  access…" action wherever the sandbox denies a folder, and a Preferences list to
  review and revoke grants. Folders dropped onto the window are usable for the session.
- **Real user folders** in Places and Go → Home, not the sandbox container.
- **Features the sandbox cannot support are absent** from the App Store edition's
  menus, palette and shortcuts: the update check, the terminal pane, running
  executables, custom Open With command lines, and everything that needs ffmpeg
  (video preview/thumbnails, subtitles, DVD images). In-process audio still plays.
  Later changes (Docker, disk images) must state their App Store availability.
- **Trash without Finder scripting** in the App Store edition (`NSFileManager`), so no
  Apple Events entitlement is needed.
- A new L0 crate `fm-macsec` wrapping the few AppKit/Foundation calls needed (open
  panel, bookmarks, security scope, `NSWorkspace` open/eject). If any of these calls
  is `unsafe` in `objc2`, the crate needs a **user-approved `unsafe` exception**
  confined to one module, like `fm-winsec`.
- A privacy manifest, App Review notes justifying each entitlement, and docs for the
  store submission.

## Capabilities

### New Capabilities
- `app-store-edition`: the sandboxed store build: feature flag, entitlements, feature
  availability, bundle contents, privacy manifest, separate settings, signing,
  packaging and upload pipeline.
- `sandbox-access`: granting, persisting, resolving, revoking and surfacing folder
  access in a sandboxed build, plus real user folders and dropped items.

### Modified Capabilities
- `app-updates`: release checks and Check for Updates do not exist in the App Store
  edition (the store updates the app).
- `build-guardrails`: the ffmpeg requirement covers the `.pkg`, and the App Store
  edition does not discover external ffmpeg.
- `security`: `fm-macsec` joins the confined-`unsafe` exceptions; sandbox entitlements
  are minimal and checked; the grants file is owner-only, bounded and treated as
  untrusted on load.
- `trash-and-undo`: the App Store edition trashes through `NSFileManager`, never by
  scripting Finder.

## Impact

- **New crate** `crates/macsec` (`fm-macsec`, L0, registered in `xtask/src/layers.rs`);
  uses `objc2`, `objc2-foundation`, `objc2-app-kit` (already in `Cargo.lock` via winit,
  so no new licences).
- **Engine**: new `access` module and `access_cmds.rs` (`access.grant`,
  `access.revoke`); a feature gate that leaves unsupported commands unregistered;
  `FmError` gains an access-not-granted variant; Places/home lookup; ffmpeg discovery
  and updates gated.
- **ops**: macOS trash method selected by edition.
- **ui / fm-app**: access-denied state in listings with a Grant button, first-run
  grant dialog, Preferences → Folder access; open panel and `NSWorkspace` calls from
  `fm-app`; About shows the edition.
- **Packaging**: `packaging/macos/` (App Store entitlements template, privacy
  manifest, Info.plist additions, review notes); `cargo xtask mas-pkg` assembles,
  signs and packages; a universal (arm64 + x86_64) binary.
- **CI/release**: a `macos-app-store` job in `release.yml` (tag and manual dispatch)
  using the existing secrets `DIST_CERT_*`, `INSTALLER_CERT_*`, `MAS_PROFILE`,
  `ASC_*`, `APPLE_TEAM_ID`; CI clippy/tests also run with `--features app-store` on macOS.
- **Docs**: `docs/development.md` (store build, sandbox testing, TestFlight),
  `docs/architecture.md` (crate, edition gate), AGENTS.md/CLAUDE.md (crate map,
  `unsafe` exception if approved), CHANGELOG.
- **Users**: App Store and DMG installs keep separate settings and bookmarks
  (`~/Library/Containers/com.filemanagerultra.mac/` vs `~/Library/Application Support/`).
