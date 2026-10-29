# branding

## Purpose

Product identity lives once in `fm-brand`; a rename tool rewrites every occurrence and a check fails on stale or hard-coded names.

## Requirements

### Requirement: Product identity has one source
All user-visible product identity — application name, slug, application id, organisation and executable name — SHALL live as constants in the `brand` crate; no other crate or app may hard-code them.

#### Scenario: Identity constants exist
- **WHEN** the `brand` crate is inspected in tests
- **THEN** it exposes `APP_NAME`, `APP_SLUG`, `APP_ID`, `ORG_NAME` and `EXECUTABLE` as non-empty constants

#### Scenario: The window title reads the brand
- **WHEN** the application window opens
- **THEN** its title is exactly `APP_NAME` (asserted against the constant, not a literal)

### Requirement: The visible name can be renamed in one commit
A rename tool SHALL rewrite every place the identity appears — brand constants, binary name, desktop entry, app id — and verify that no stale name survives.

#### Scenario: Rename end to end
- **WHEN** `cargo xtask rename "NewUltra"` runs
- **THEN** the brand constants, the app binary name and the desktop-file/app-id metadata are updated
- **AND** `cargo xtask brand-check` passes immediately afterwards

#### Scenario: Rename is idempotent
- **WHEN** `cargo xtask rename` is run twice with the same name
- **THEN** the second run changes nothing and exits 0

### Requirement: Stale names fail the build
A check SHALL fail when the previous product name (or any name recorded in the rename tool's history) appears outside `openspec/`, `log/` and the `brand` crate itself.

#### Scenario: A hard-coded name is caught
- **WHEN** a literal product name is added to a UI string
- **THEN** `cargo xtask brand-check` fails and names the file and line

#### Scenario: Legitimate mentions are allowed
- **WHEN** the name appears in the brand crate, changelogs or archived specs
- **THEN** the check passes

### Requirement: Identity is available to installers and desktop integration
The desktop entry, file-association metadata and window class SHALL derive from `brand` constants rather than duplicated strings.

#### Scenario: Desktop file generation
- **WHEN** packaging metadata is generated (later changes build on this)
- **THEN** its `Name`, `Exec` and `StartupWMClass` fields come from `APP_NAME`, `EXECUTABLE` and `APP_SLUG`

### Requirement: Recognisable platform-specific application icon
The application icon SHALL be the approved split folder with a shallow gold tab,
yellow left panel, blue right panel and a dark central seam. Linux SHALL use a flat
free silhouette, Windows a subtly shaded free silhouette, and macOS an inset
symbol on a light rounded-square tile. Window/About icons and platform packaging
SHALL use the corresponding treatment, with editable sources owned by `fm-brand`.

#### Scenario: Consistent installed and running identity
- **WHEN** the app runs or is installed on Linux, Windows or macOS
- **THEN** its window/About icon and launcher/bundle icon use the same platform treatment
- **AND** Windows ICO includes 16 through 256 px representations and macOS ICNS includes standard and Retina representations through 1024 px
