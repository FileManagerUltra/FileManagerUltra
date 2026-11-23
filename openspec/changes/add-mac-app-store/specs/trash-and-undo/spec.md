## ADDED Requirements

### Requirement: App Store trash does not script Finder
In the App Store edition, `ops::trash` SHALL move items to the Trash through
`NSFileManager` and SHALL NOT script Finder or send Apple Events. Items so trashed may
lack Finder's Put Back; undo of a trash stays unavailable on macOS as before. The DMG
build SHALL keep its current trash method.

#### Scenario: Trash in the sandbox
- **WHEN** the user deletes a file in a granted folder in the App Store edition
- **THEN** the file appears in the Trash and macOS shows no automation-permission prompt
