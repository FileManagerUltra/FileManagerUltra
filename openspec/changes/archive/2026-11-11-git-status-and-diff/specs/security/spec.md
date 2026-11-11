## ADDED Requirements

### Requirement: Hostile repositories are inert
Opening a repository for status or diff SHALL never execute hooks, filter drivers, fsmonitor,
credential helpers, textconv or any external program, and SHALL ignore configuration that would
reference them. Repository paths, ref names, file names and blob contents are untrusted:
discovery depth, entry counts, blob sizes, ref lookups and diff input sizes are capped, and
symlinks are never followed out of the working tree.

#### Scenario: Malicious config
- **WHEN** a repo's config sets `core.fsmonitor` or a filter driver to a command
- **THEN** status and diff complete without running it

#### Scenario: Filter driver
- **WHEN** a repo's config defines `filter.<name>.clean`/`process` and `.gitattributes` applies it
- **THEN** repository-local `filter.*` sections are removed from the in-memory config and nothing runs

#### Scenario: Oversized or corrupt data
- **WHEN** a repo has a corrupt object, a 10 GiB blob or 1 000 000 untracked files
- **THEN** the feature degrades with a visible notice, bounded memory and no panic
