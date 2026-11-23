## ADDED Requirements

### Requirement: The App Store edition is a build flavour of the same app
The workspace SHALL build an App Store edition from the same sources by enabling the
`app-store` cargo feature on `fm-app`, which SHALL propagate to every crate whose
behaviour differs. The feature SHALL only change behaviour on macOS; on other targets
it SHALL compile and behave like the default build. Default builds SHALL be unchanged
by this change apart from the shared code paths it refactors. About SHALL name the
edition ("App Store edition") when the feature is on.

#### Scenario: Both flavours build and pass the gate
- **WHEN** CI runs clippy and the tests on macOS with and without `--features app-store`
- **THEN** both pass with warnings denied

#### Scenario: Edition is visible
- **WHEN** the user opens About in the App Store edition
- **THEN** it shows the product name, version and "App Store edition"

### Requirement: Features the sandbox cannot support are absent, not broken
In the App Store edition the following SHALL NOT be registered as commands, and SHALL
therefore not appear in menus, the command palette, context menus or shortcut tables:
checking for updates, the terminal pane, running an executable file, Open With entries
that are custom command lines, and every feature that needs an external `ffmpeg`
(video preview and thumbnails, subtitle extraction, DVD/ISO video). In-process audio
playback, PDF, Markdown, images, text, archives already supported in-process, git
status (in-process) and SMB/SFTP SHALL remain available. Dispatching an unregistered id
(for example from a stored workspace or shortcut) SHALL return an error, never panic.
A workspace that contains a terminal pane SHALL load with that pane replaced by a
Files pane.

#### Scenario: Menu has no unsupported items
- **WHEN** the App Store edition builds its menus and palette
- **THEN** no entry for Check for Updates, New Terminal, Run, a custom Open With command or a video-only action exists

#### Scenario: Stale dispatch
- **WHEN** an unregistered command id such as `term.open` is dispatched in the App Store edition
- **THEN** dispatch returns an unknown-command error and the app keeps running

#### Scenario: Workspace from the DMG build
- **WHEN** a saved workspace layout with a terminal pane is loaded by the App Store edition
- **THEN** the pane opens as a Files pane at the real home folder and no error dialog appears

#### Scenario: A video file in the App Store edition
- **WHEN** the user previews a video file
- **THEN** the preview shows the file's metadata and says video playback is not available in the App Store edition, without suggesting installing ffmpeg

### Requirement: Opening files uses Launch Services
Opening a file in the App Store edition SHALL go, for its default application and for
Open With a listed application, through `NSWorkspace`, not by spawning a process. A file
the user can see in a granted folder SHALL open in the chosen app.

#### Scenario: Open with default app
- **WHEN** the user opens a PDF in a granted folder
- **THEN** it opens in the default PDF app and no child process of the file manager is spawned

### Requirement: Sandbox entitlements are minimal and fixed
The App Store edition SHALL be signed with exactly these entitlements:
`com.apple.security.app-sandbox`, `com.apple.security.files.user-selected.read-write`,
`com.apple.security.files.bookmarks.app-scope`, `com.apple.security.network.client`,
`com.apple.security.network.server` (local network discovery), and the
`com.apple.application-identifier` and `com.apple.developer.team-identifier` values
matching the provisioning profile. It SHALL NOT use any
`com.apple.security.temporary-exception.*`, Apple Events or absolute-path entitlement.
The entitlements file SHALL be generated from a checked-in template, with the team id
taken from the build environment, and a test SHALL fail if any other key appears.

#### Scenario: Template is checked
- **WHEN** `cargo test -p xtask` runs
- **THEN** a test parses the generated entitlements and fails on any key outside the allowed list or any `temporary-exception` key

#### Scenario: Signed bundle is verified
- **WHEN** the release job has signed the app
- **THEN** `codesign -d --entitlements -` shows exactly the allowed keys and the job fails otherwise

### Requirement: The store bundle is complete and self-describing
The App Store `.app` SHALL be a universal (arm64 and x86_64) binary, SHALL embed the
Mac App Store provisioning profile as `Contents/embedded.provisionprofile`, and its
`Info.plist` SHALL carry the bundle id from `fm-brand`, `CFBundleShortVersionString`
and `CFBundleVersion` equal to the workspace version,
`LSApplicationCategoryType = public.app-category.utilities`,
`ITSAppUsesNonExemptEncryption = false`, `NSLocalNetworkUsageDescription` and the
Bonjour service types used by discovery. The bundle SHALL include a
`PrivacyInfo.xcprivacy` declaring no tracking, no collected data and the
required-reason APIs it uses. It SHALL NOT contain `ffmpeg`, `ffprobe` or any other
executable besides the app binary.

#### Scenario: Bundle contents check
- **WHEN** `cargo xtask mas-pkg` finishes
- **THEN** it verifies with `lipo` that both architectures are present, that the embedded profile's application identifier matches the bundle id, and that `Contents/MacOS` holds exactly one executable, and fails otherwise

### Requirement: Store settings are separate from the DMG build
The App Store edition SHALL keep preferences, workspaces, known hosts and grants
inside its sandbox container and SHALL NOT read or migrate files of a DMG install.
The documentation SHALL say that the two installs do not share settings.

#### Scenario: Both editions installed
- **WHEN** a user runs the DMG build and the App Store build on one Mac
- **THEN** changing a preference in one does not change it in the other and neither crashes

### Requirement: Release pipeline packages and uploads, never submits
The release workflow SHALL, for a version tag and for a manual dispatch, build the
App Store edition on a macOS runner, import the Apple Distribution and Mac Installer
Distribution certificates into a temporary keychain that is deleted afterwards,
install the provisioning profile, sign the app and the `.pkg`, validate the package
with App Store Connect and upload it. It SHALL NOT submit the build for review; that
stays a manual step in App Store Connect. Without the signing secrets the job SHALL
skip with a message, not fail other release jobs. A manual dispatch SHALL be able to
validate without uploading. Secrets SHALL only be written to the runner's temporary
directory and SHALL NOT be printed.

#### Scenario: Tagged release
- **WHEN** tag `vX.Y.Z` is pushed and the secrets exist
- **THEN** a signed `.pkg` for version `X.Y.Z` is uploaded to App Store Connect and appears there for TestFlight and review

#### Scenario: Dry run
- **WHEN** the workflow is dispatched manually with the App Store target and validate-only set
- **THEN** the package is built, signed and validated, and nothing is uploaded

#### Scenario: Missing secrets
- **WHEN** the App Store signing secrets are absent (for example in a fork)
- **THEN** the App Store job reports that it was skipped and the DMG, deb and Windows jobs still run
