## MODIFIED Requirements

### Requirement: Panics and unsafe code are denied in library code
Workspace lints SHALL deny `unwrap`, `expect`, `panic`, `todo`, `unimplemented` and `unreachable` in non-test code, and `unsafe_code = "forbid"` workspace-wide. They SHALL also deny unchecked indexing and slicing (`clippy::indexing_slicing`) and byte-range slicing of `str` (`clippy::string_slice`) in non-test code, so hostile filenames, listings and documents cannot panic through an out-of-range index or a split UTF-8 character.

#### Scenario: A panic path fails the build
- **WHEN** non-test code adds an `unwrap()` or `panic!()`
- **THEN** `cargo clippy --workspace --all-targets -- -D warnings` fails

#### Scenario: Tests keep their escape hatch
- **WHEN** test code uses `unwrap()` in `#[cfg(test)]` modules and integration tests
- **THEN** the build still passes (the lint is configured for non-test code only)

#### Scenario: Unchecked indexing fails the build
- **WHEN** non-test code indexes a slice with `v[i]`, slices bytes with `&b[a..b]` or slices a `str` with `&s[a..b]`
- **THEN** clippy fails and the code must use `get()`, a slice pattern, `split_once`/`strip_prefix` or an iterator

### Requirement: One task runs the full gate
`cargo xtask ci` SHALL run formatting check, clippy (all targets and features, locked), tests, layering, the brand/token checks, the dependency policy and the unused-dependency check, and fail on the first problem with an actionable message.

#### Scenario: A green tree passes
- **WHEN** `cargo xtask ci` runs on a healthy workspace
- **THEN** it exits 0 and prints each step's result

#### Scenario: A broken step fails the run
- **WHEN** any individual step fails
- **THEN** `cargo xtask ci` exits non-zero, naming the step and the command to reproduce it

#### Scenario: A missing tool is named up front
- **WHEN** `cargo-deny` or `cargo-machete` is not installed
- **THEN** `cargo xtask ci` fails before the long steps and prints the install command

## ADDED Requirements

### Requirement: Dependencies follow a checked policy
`deny.toml` SHALL allow only permissive licences, with any other licence admitted as an exception scoped to one crate after the user's approval (today: EPL-2.0 for `merman-elk-layered`). RustSec vulnerabilities, unmaintained-crate advisories and yanked crates SHALL fail the check unless ignored with a written reason, and dependencies SHALL come from crates.io only. Workspace crates outside `vendor/` SHALL have no unused dependencies.

#### Scenario: A new copyleft dependency
- **WHEN** a dependency under a licence outside the allow-list is added
- **THEN** `cargo deny check` fails and names the crate and licence

#### Scenario: A new advisory
- **WHEN** RustSec publishes an advisory for a locked crate
- **THEN** the next CI run fails until the crate is updated or the advisory is ignored with a reason

### Requirement: CI runs the gate on every push and pull request
A GitHub workflow SHALL run on every push to `main` and every pull request: format, clippy with warnings denied on Linux, Windows and macOS, layers, branding, the dependency policy, the unused-dependency check, tests and doctests on Linux. Pixel snapshots and the mess report SHALL be informational and SHALL NOT fail the run. The workflow SHALL use only actions on the repository's allow-list.

#### Scenario: A pull request with a clippy warning
- **WHEN** a pull request introduces a clippy warning
- **THEN** the `ci` workflow's lint job fails on that PR

### Requirement: Git hooks run the fast checks locally
`cargo xtask hooks` SHALL point git at the versioned `.githooks/` folder, whose scripts delegate to `cargo xtask hook <name>`. The pre-commit hook SHALL check formatting, layering and branding; the pre-push hook SHALL run workspace clippy with warnings denied and, when installed, the dependency checks. Hooks SHALL be skippable once with `--no-verify` and removable with `cargo xtask hooks uninstall`.

#### Scenario: Committing unformatted code
- **WHEN** a developer with the hooks installed commits code that `cargo fmt` would change
- **THEN** the commit is refused with the command that fixes it

### Requirement: A mess report shows code-health hotspots
`cargo xtask mess` SHALL list the longest and most complex functions and bool-heavy signatures, largest first, and SHALL NOT fail on findings. CI SHALL publish it in the run summary.

#### Scenario: Report only
- **WHEN** the tree contains functions over the length limit
- **THEN** `cargo xtask mess` lists them and exits 0
