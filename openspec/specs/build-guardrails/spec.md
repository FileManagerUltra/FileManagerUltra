# build-guardrails

## Purpose

Layering, panic/unsafe bans and the single CI task that runs the whole gate.

## Requirements

### Requirement: Layering is enforced by tooling
The workspace SHALL declare a layer for every crate (L0 foundation … L5 app shell), and `cargo xtask layers` SHALL fail when a crate depends on a crate from a higher layer or an unregistered crate appears.

#### Scenario: A bad dependency fails the check
- **WHEN** a lower-layer crate adds a dependency on a higher-layer crate
- **THEN** `cargo xtask layers` exits non-zero and names both crates and their layers

#### Scenario: A new crate must be registered
- **WHEN** a crate directory exists that is not listed in `xtask`'s layer table
- **THEN** `cargo xtask layers` fails and tells the contributor to register it

#### Scenario: The current tree passes
- **WHEN** `cargo xtask layers` runs on the scaffolded workspace
- **THEN** it exits 0

### Requirement: Panics and unsafe code are denied in library code
Workspace lints SHALL deny `unwrap`, `expect`, `panic`, `todo`, `unimplemented` and `unreachable` in non-test code, and `unsafe_code = "forbid"` workspace-wide.

#### Scenario: A panic path fails the build
- **WHEN** non-test code adds an `unwrap()` or `panic!()`
- **THEN** `cargo clippy --workspace --all-targets -- -D warnings` fails

#### Scenario: Tests keep their escape hatch
- **WHEN** test code uses `unwrap()` in `#[cfg(test)]` modules and integration tests
- **THEN** the build still passes (the lint is configured for non-test code only)

### Requirement: One task runs the full gate
`cargo xtask ci` SHALL run formatting check, clippy, tests, layering and the brand/token checks, and fail on the first problem with an actionable message.

#### Scenario: A green tree passes
- **WHEN** `cargo xtask ci` runs on a healthy workspace
- **THEN** it exits 0 and prints each step's result

#### Scenario: A broken step fails the run
- **WHEN** any individual step fails
- **THEN** `cargo xtask ci` exits non-zero, naming the step and the command to reproduce it
