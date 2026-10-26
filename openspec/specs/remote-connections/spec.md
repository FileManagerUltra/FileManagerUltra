# remote-connections Specification

## Purpose
TBD - created by archiving change sftp-backend. Update Purpose after archive.
## Requirements
### Requirement: Typed addresses, forgetting hosts and key formats
Typing an `sftp://` address with no registered connection SHALL switch the pane to the SFTP type and show the connect form for that server. Trusted host keys SHALL be listed in Preferences and be removable through `remote.hostkey.forget`; the next connect SHALL ask again. `known_hosts.json` SHALL be re-read on every connect. PuTTY `.ppk` keys SHALL be refused with a message explaining how to export an OpenSSH key. Poll/keepalive/operation timeouts SHALL be configurable in Preferences.

#### Scenario: Forget a host
- **WHEN** the user forgets a trusted host and connects again
- **THEN** the host-key trust dialog is shown again

#### Scenario: PuTTY key
- **WHEN** the selected key file ends in `.ppk`
- **THEN** connecting fails with an actionable message and no network access

