## ADDED Requirements

### Requirement: Removal runs fixed tools with listed arguments only
Eject and Dismount SHALL execute fixed per-OS adapters with argument arrays and SHALL derive the device identity from the scanned list, not from command parameters. No untrusted text may be interpolated into shell or PowerShell source; any required script SHALL be fixed with separately validated arguments. Device labels and mount paths are untrusted: display copies are length-capped and stripped of control characters, while operation identities are preserved separately and never interpreted as commands or markup. Removal SHALL verify the current mount identity before acting so a reused mount point cannot target a replacement volume.

#### Scenario: Hostile volume label
- **WHEN** a device label contains control characters, a newline or shell metacharacters
- **THEN** it is shown sanitised and has no effect on any executed command
