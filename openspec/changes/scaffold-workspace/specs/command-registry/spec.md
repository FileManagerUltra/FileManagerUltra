# command-registry

## ADDED Requirements

### Requirement: Every user-visible action is a registered command
All behaviour reachable from the UI SHALL be defined as a `CommandSpec` in the engine with an id, label, menu path, optional shortcut, parameter documentation, an `enabled` predicate and a `run` implementation.

#### Scenario: Seed commands exist after scaffolding
- **WHEN** the engine registry is built in tests
- **THEN** it contains at least `app.quit`, `app.about`, `view.theme.dark`, `view.theme.light`, `view.theme.system`, `view.theme.toggle`
- **AND** each entry has a unique id, a non-empty label and a menu path

#### Scenario: A command with no implementation is impossible
- **WHEN** a new `CommandSpec` is added without a `run` function
- **THEN** it does not compile (the field is not optional in the type)

### Requirement: Dispatch is the single entry point for behaviour
Menus, shortcuts and every future caller SHALL reach behaviour only through `Session::dispatch(id, params)`; UI code SHALL NOT call backend or operation functions directly.

#### Scenario: Dispatch by id
- **WHEN** `Session::dispatch("view.theme.toggle", …)` is called
- **THEN** the toggle command's `run` executes and returns `Ok(())` on success

#### Scenario: Unknown id is an error, not a crash
- **WHEN** `Session::dispatch("does.not.exist", …)` is called
- **THEN** it returns `Err(FmError::…)` naming the id, and the session is unchanged

### Requirement: Commands never panic on bad input
A command's `run` SHALL return `Err` for any parameters it does not understand — missing, wrongly typed, out of range or absurdly large — for any session state.

#### Scenario: Malformed parameters
- **WHEN** each registered command is invoked with empty, wrong-typed and oversized parameter payloads in a fuzz test
- **THEN** every call returns `Ok` or `Err`, never panics, and the process keeps running

### Requirement: Shortcuts and menus reference only real commands
The menu catalogue and keyboard-shortcut table SHALL be checked against the registry, so a renamed or removed command cannot leave a dead menu item or shortcut behind.

#### Scenario: Dead reference fails the gate
- **WHEN** a menu entry or shortcut names a command id that is not in the registry
- **THEN** `cargo xtask brand-check`-equivalent validation (part of the CI task) fails with the offending id

### Requirement: Long actions run as jobs, not inline
A command that cannot finish within one frame SHALL enqueue a background job and return `Ok(())` immediately; progress and completion arrive through the job queue.

#### Scenario: Scaffolded queue accepts jobs
- **WHEN** a test command enqueues a trivial job
- **THEN** the UI-facing poll returns its progress events and a terminal `Done` result, and the calling command returned before the job finished
