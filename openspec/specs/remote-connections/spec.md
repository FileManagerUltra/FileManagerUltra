# remote-connections Specification

## Purpose
TBD - created by archiving change sftp-backend. Update Purpose after archive.
## Requirements
### Requirement: SFTP failures are actionable in the connect dialog
Failed SFTP connections initiated by opening a server SHALL show an error in
the connection dialog, including silent attempts with remembered credentials.
The dialog SHALL preserve non-secret server/profile fields and allow retrying.
Unreachable-server and timeout errors SHALL suggest checking the address,
network or VPN, and firewall without claiming a particular network cause.
Form validation errors SHALL also remain in the dialog.

#### Scenario: Remembered login cannot reach the NAS
- **WHEN** a remembered-password connection fails because the server is unreachable
- **THEN** a prefilled connection dialog opens with the error and a Connect button
- **AND** the error is not shown only in the status bar

#### Scenario: Opening an unconnected saved server
- **WHEN** a user opens an SFTP server row without an existing backend
- **THEN** navigation starts the connection flow without an unsupported-backend error
- **AND** the current pane location is preserved until the connection succeeds

#### Scenario: User cancelled an explicit connection dialog
- **WHEN** an in-flight connection fails after its explicit dialog was cancelled
- **THEN** the cancelled dialog is not reopened and the error is reported in the status bar

### Requirement: Typed addresses, forgetting hosts and key formats
Typing an `sftp://` address with no registered connection SHALL switch the pane to the SFTP type and show the connect form for that server. Trusted host keys SHALL be listed in Preferences and be removable through `remote.hostkey.forget`; the next connect SHALL ask again. `known_hosts.json` SHALL be re-read on every connect. PuTTY `.ppk` keys SHALL be refused with a message explaining how to export an OpenSSH key. Poll/keepalive/operation timeouts SHALL be configurable in Preferences.

#### Scenario: Forget a host
- **WHEN** the user forgets a trusted host and connects again
- **THEN** the host-key trust dialog is shown again

#### Scenario: PuTTY key
- **WHEN** the selected key file ends in `.ppk`
- **THEN** connecting fails with an actionable message and no network access

### Requirement: Remote connections are protocol-aware
`remote.connect` SHALL accept a `protocol` (`sftp` default, `smb`). SMB connections use the same per-authority state machine, stale banner, Disconnect, reconnect, poll refresh, profiles and cross-backend operations as SFTP, with no host-key step. SMB profiles SHALL additionally hold `domain` and `share` (never a password), and profiles saved before this change SHALL load as SFTP.

#### Scenario: Connect to a share
- **WHEN** the user fills host, user, password and share in the Connect to SMB Server popup
- **THEN** the pane lists the share, the Disconnect button appears, and the password is not written to disk

#### Scenario: Old profile
- **WHEN** `prefs.toml` contains a profile without `protocol`
- **THEN** it loads as an SFTP profile unchanged

### Requirement: Typed smb addresses and restore
Typing an `smb://` address with no registered connection SHALL open the Authentication Required popup, prefilled with the user from the address when it has one.

#### Scenario: Typed address
- **WHEN** the user types `smb://alice@nas/media` in the location bar
- **THEN** the popup opens with `alice` prefilled and connects to `nas` on submit

### Requirement: Neutral timing preferences
Remote timeouts and poll interval SHALL apply to both protocols under `remote_*` preference keys; previous `sftp_*` keys SHALL still be read.

#### Scenario: Old preferences
- **WHEN** `prefs.toml` has `sftp_poll_secs = 10`
- **THEN** the remote poll interval is 10 s
