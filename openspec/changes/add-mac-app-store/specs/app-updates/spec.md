## MODIFIED Requirements

### Requirement: Non-blocking release checks
The application SHALL check the official repository's latest published stable
release once at startup by default, on a dedicated engine worker, without
blocking directory jobs or the UI. Preferences SHALL allow disabling startup
checks. Help SHALL offer Check for Updates independently of this preference.
The App Store edition SHALL NOT contain release checks, the Check for Updates
command, the update notification or the preference, and SHALL make no request to
GitHub, because the Mac App Store updates it.

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

#### Scenario: App Store edition
- **WHEN** the App Store edition starts and the user opens the Help menu and Preferences
- **THEN** no release request is made, Help has no Check for Updates entry and Preferences has no update setting
