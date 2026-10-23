## ADDED Requirements

### Requirement: Terminal child hygiene
The terminal child SHALL inherit the user's environment minus app-internal variables (`FM_*`, `CARGO_*`, `RUST_BACKTRACE`), run as the current user only, and be terminated on pane close and app exit. Terminal output and input MUST NOT be written to logs, devlog or state files, and `Prefs` stores no terminal content.

#### Scenario: No content on disk
- **WHEN** a terminal session prints text and the app saves state
- **THEN** the state files contain none of that text

#### Scenario: Injection via directory name
- **WHEN** `term.cd_here` targets a directory named `x; rm -rf ~`
- **THEN** the name is quoted so it is a single literal argument
