## ADDED Requirements

### Requirement: Drives are shown without the folder slash
The listing SHALL mark folders with a trailing `/`, except Windows drive roots (the entries of This PC), which SHALL be shown as Windows writes them: `C:` or `Label (C:)`. Preview's folder table for This PC SHALL follow the same rule.

#### Scenario: This PC on Windows
- **WHEN** a Windows user opens This PC with drives `C:` (labelled Windows) and `D:` (labelled Data)
- **THEN** the rows read `Windows (C:)` and `Data (D:)`, with no trailing slash, while folders inside a drive still end in `/`
