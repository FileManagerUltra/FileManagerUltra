## ADDED Requirements

### Requirement: Docker access is local, fixed and non-invasive
The app SHALL talk only to a local engine socket or named pipe, SHALL use only the Engine API endpoints for listing, inspecting, the container archive endpoints, creating and removing its own labelled helper containers and (after an explicit choice) pulling the helper image, and SHALL NOT start, stop, remove, exec into or change user containers, images or volumes other than through the file operations the user performs. It SHALL NOT run the `docker` CLI.

#### Scenario: No exec
- **WHEN** any command path is exercised in the test suite with a recording fake engine
- **THEN** no request to `/exec`, `/start`, `/stop`, `/kill` or `DELETE` of a container without the app's label is made

### Requirement: Engine responses are untrusted and bounded
HTTP responses from the engine SHALL be parsed with caps on header size and count, body sizes and JSON depth; tar streams from containers SHALL be read with the archive limits and name normalisation; a malformed or hostile response SHALL produce an error, never a crash or unbounded allocation.

#### Scenario: Endless tar stream
- **WHEN** a fake engine answers a directory listing with an endless tar stream
- **THEN** listing stops at the entry and byte caps with an actionable error
