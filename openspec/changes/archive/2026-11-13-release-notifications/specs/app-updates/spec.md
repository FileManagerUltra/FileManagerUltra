## ADDED Requirements

### Requirement: Non-blocking release checks
The application SHALL check the official repository's latest published stable
release once at startup by default, on a dedicated engine worker, without
blocking directory jobs or the UI. Preferences SHALL allow disabling startup
checks. Help SHALL offer Check for Updates independently of this preference.

#### Scenario: An update exists
- **WHEN** a published stable release has a greater semantic version
- **THEN** the Jobs panel opens with a dismissible update entry showing its version and a download button
- **AND** the app remains usable and does not restart
- **AND** existing file operation entries remain visible and functional

#### Scenario: No release or no newer release
- **WHEN** GitHub returns no release or a release not newer than the running version
- **THEN** startup checks show no notification
- **AND** a manual check reports that no newer published release is available
- **AND** neither checking nor no-update results open the Jobs panel

#### Scenario: Check fails
- **WHEN** a request times out, is rate-limited, or returns malformed or oversized metadata
- **THEN** the status bar reports the error without opening the Jobs panel
- **AND** the Help command allows retrying without blocking app use

### Requirement: User-controlled installation
The download command SHALL open the official GitHub release page in the browser
only after an explicit user action. The notification SHALL explain that
installation is manual. The application SHALL NOT execute downloaded code.

#### Scenario: Download is chosen
- **WHEN** the user chooses Download update
- **THEN** the browser opens the official release page, not an arbitrary metadata URL

### Requirement: Bounded release metadata
Release checks SHALL use HTTPS with certificate validation, disallow redirects,
limit metadata to 1 MiB and total network time to 15 seconds, ignore draft and
prerelease versions, and coalesce concurrent requests.

#### Scenario: An untrusted response is received
- **WHEN** release metadata is invalid or exceeds 1 MiB
- **THEN** the check fails visibly and no URL from that response is opened
