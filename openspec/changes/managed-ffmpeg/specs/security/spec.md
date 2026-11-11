## ADDED Requirements

### Requirement: Downloaded helper tools are pinned and verified
Any executable the app downloads at the user's request SHALL come from a pin compiled into the app (HTTPS URL, exact size, SHA-256, expected archive members). The download SHALL be rejected, and nothing unpacked or executed, unless its size and SHA-256 match the pin. Pins are never fetched from the network, and a "latest" URL is never used.

#### Scenario: Wrong hash
- **WHEN** the downloaded bytes do not match the pinned SHA-256
- **THEN** installation fails with an actionable error, no tool file is created or replaced, and temporary files are removed

#### Scenario: Oversized or endless download
- **WHEN** the response is larger than the pinned size or the hard cap
- **THEN** reading stops at the cap and installation fails without writing the rest

#### Scenario: Hostile archive
- **WHEN** the verified archive contains entries with `..` or absolute paths, symlinks, duplicate or extra members, or members above the per-file cap
- **THEN** only the exactly-named expected members are extracted, to fixed file names chosen by the app, and any other entry is ignored or rejects the install

#### Scenario: Safe install location
- **WHEN** the tools are installed
- **THEN** they are written to a temporary file in the app's tools folder, then renamed into place; the folder is owner-only on Unix and the manifest records only the version and hashes (no secrets)

#### Scenario: HTTPS only
- **WHEN** the download or any redirect uses a scheme other than `https`
- **THEN** the request is refused
