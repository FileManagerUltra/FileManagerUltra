## ADDED Requirements

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
