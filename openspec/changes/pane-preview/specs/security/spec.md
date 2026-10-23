## ADDED Requirements

### Requirement: Previews are untrusted-input safe
File content shown in a Preview SHALL be treated as hostile: bounded reads, header checks before allocation, no network access, no execution of embedded content, decoder panics contained, and decoder child processes (video) started without a shell and with fixed arguments, the file passed as a single argument after `--`/via `file:` protocol guard.

#### Scenario: Hostile filename to the decoder
- **WHEN** a video file is named `-i evil.mp4` or contains shell metacharacters
- **THEN** the decoder receives it as one argument and no other file or command is touched
