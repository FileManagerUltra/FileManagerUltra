## ADDED Requirements

### Requirement: Connecting is a command that never blocks the UI
`remote.connect {host, port?, user, auth, start_dir?, profile?}` SHALL open the connection on the `sftp` lane and report progress through the job queue. `auth` is a password or a private-key path with optional passphrase. On success the backend SHALL be registered under its `(scheme, authority)` and the focused pane SHALL navigate to the start directory (default: the server's home). On failure the pane SHALL stay untouched and the dialog SHALL show the reason with a retry.

#### Scenario: Successful connect
- **WHEN** `remote.connect` succeeds against the test server
- **THEN** the focused pane's location is `sftp://user@host:port/<dir>` and its listing fills through the normal `DirCache` path

#### Scenario: Bad parameters
- **WHEN** `remote.connect` receives a missing host, a port above 65535 or an oversized field
- **THEN** it returns `Err` and nothing is attempted

#### Scenario: Wrong password
- **WHEN** authentication fails
- **THEN** the dialog shows "authentication failed" without echoing the secret, and the pane is unchanged

### Requirement: Host keys are trusted on first use, explicitly
The first connection to a host SHALL show the key fingerprint and require confirmation before it is saved to the known-hosts file; a later connection with a different key SHALL be refused with an error naming the stored and offered fingerprints and SHALL NOT offer a one-click accept.

#### Scenario: First use
- **WHEN** a host is unknown
- **THEN** the connect pauses on a fingerprint confirm and only a confirm saves the key

#### Scenario: Changed key
- **WHEN** the offered key differs from the stored one
- **THEN** the connection is refused and nothing is written to known hosts

### Requirement: Connection state is visible and mutations are guarded
Each remote authority SHALL have a state `Connecting`, `Ready` or `Failed(reason)` shown as a badge in the pane chrome. While not `Ready` the last listing SHALL stay visible marked stale, mutating commands for that pane SHALL be disabled, and `remote.reconnect` SHALL retry with the in-memory credentials still held (or prompt again).

#### Scenario: Connection drops
- **WHEN** the server closes the connection while a listing is shown
- **THEN** the badge shows the failure, the listing stays visible and stale, and F5/F6/F8 are disabled for that pane until reconnect

### Requirement: Profiles save everything except secrets
Named profiles (name, host, port, user, key file path, start directory) SHALL persist in `prefs.toml` and be chosen from the connect dialog. Passwords and passphrases SHALL NOT be stored in any profile or file; the dialog SHALL say that they are kept for the session only.

#### Scenario: Profile round trip
- **WHEN** a profile is saved and the app restarts
- **THEN** the profile is listed with its fields and the password field is empty

### Requirement: Remote panes refresh without a watcher
A visible remote pane SHALL refresh on focus regain, on `nav.refresh`, and every 5 seconds while visible, reusing the directory cache so several panes on one directory share one read; a hidden pane SHALL NOT poll.

#### Scenario: Poll
- **WHEN** a file appears on the server while the pane is visible
- **THEN** it shows up within about 5 seconds without user action

### Requirement: The status bar shows free space
The status bar SHALL show free and total space for the focused location from `Vfs::space` (local and SFTP), computed on the pane's lane and cached, and SHALL show nothing when the backend does not report it.

#### Scenario: Space is unavailable
- **WHEN** the server does not support the statvfs extension
- **THEN** the status bar omits the space field and no error is shown

## ADDED Requirements

### Requirement: Typed addresses, forgetting hosts and key formats
Typing an `sftp://` address with no registered connection SHALL switch the pane to the SFTP type and show the connect form for that server. Trusted host keys SHALL be listed in Preferences and be removable through `remote.hostkey.forget`; the next connect SHALL ask again. `known_hosts.json` SHALL be re-read on every connect. PuTTY `.ppk` keys SHALL be refused with a message explaining how to export an OpenSSH key. Poll/keepalive/operation timeouts SHALL be configurable in Preferences.

#### Scenario: Forget a host
- **WHEN** the user forgets a trusted host and connects again
- **THEN** the host-key trust dialog is shown again

#### Scenario: PuTTY key
- **WHEN** the selected key file ends in `.ppk`
- **THEN** connecting fails with an actionable message and no network access
