# build-guardrails

## Purpose

Layering, panic/unsafe bans and the single CI task that runs the whole gate.
## Requirements
### Requirement: Layering is enforced by tooling
The workspace SHALL declare a layer for every crate (L0 foundation … L5 app shell), and `cargo xtask layers` SHALL fail when a crate depends on a crate from a higher layer or an unregistered crate appears.

#### Scenario: A bad dependency fails the check
- **WHEN** a lower-layer crate adds a dependency on a higher-layer crate
- **THEN** `cargo xtask layers` exits non-zero and names both crates and their layers

#### Scenario: A new crate must be registered
- **WHEN** a crate directory exists that is not listed in `xtask`'s layer table
- **THEN** `cargo xtask layers` fails and tells the contributor to register it

#### Scenario: The current tree passes
- **WHEN** `cargo xtask layers` runs on the scaffolded workspace
- **THEN** it exits 0

### Requirement: Panics and unsafe code are denied in library code
Workspace lints SHALL deny `unwrap`, `expect`, `panic`, `todo`, `unimplemented` and `unreachable` in non-test code, and `unsafe_code = "forbid"` workspace-wide.

#### Scenario: A panic path fails the build
- **WHEN** non-test code adds an `unwrap()` or `panic!()`
- **THEN** `cargo clippy --workspace --all-targets -- -D warnings` fails

#### Scenario: Tests keep their escape hatch
- **WHEN** test code uses `unwrap()` in `#[cfg(test)]` modules and integration tests
- **THEN** the build still passes (the lint is configured for non-test code only)

### Requirement: One task runs the full gate
`cargo xtask ci` SHALL run formatting check, clippy, tests, layering and the brand/token checks, and fail on the first problem with an actionable message.

#### Scenario: A green tree passes
- **WHEN** `cargo xtask ci` runs on a healthy workspace
- **THEN** it exits 0 and prints each step's result

#### Scenario: A broken step fails the run
- **WHEN** any individual step fails
- **THEN** `cargo xtask ci` exits non-zero, naming the step and the command to reproduce it

### Requirement: ffmpeg is discovered, not bundled, and audio output has a documented system dependency
Installers for Linux, Windows and macOS SHALL NOT bundle `ffmpeg` or `ffprobe`. The app SHALL discover them next to the executable, on `PATH` or in package-manager locations, and, where a verified build is pinned (Windows), in the app-managed folder filled by the checksum-verified download specified by the `managed-ffmpeg` change. Audio-only playback of formats decoded in-process SHALL NOT require ffmpeg. The audio output crate's Linux build dependency (ALSA development headers) SHALL be installed by CI and documented in `docs/development.md` as an approved exception to the "no system libraries" rule; the runtime SHALL work without extra packages on desktops that have ALSA, PipeWire or PulseAudio.

#### Scenario: Fresh install without ffmpeg
- **WHEN** an installed app starts on a machine without ffmpeg
- **THEN** in-process audio formats play, and a video reports that ffmpeg is missing with an actionable hint (on Windows, the offer to download the pinned build)

#### Scenario: Installer contents
- **WHEN** the release workflow builds the `.deb`/AppImage, NSIS installer and `.dmg`
- **THEN** none of them contains an `ffmpeg` or `ffprobe` binary

#### Scenario: Linux runtime libraries
- **WHEN** the `.deb` is built
- **THEN** its `Depends` lists the binary's runtime libraries (`libc6 (>= 2.35)`, `libgcc-s1`, `libasound2t64 | libasound2`), so apt installs ALSA where it is missing; the AppImage leaves `libasound.so.2` to the host, as the AppImage excludelist does

