## ADDED Requirements

### Requirement: Docker entry in Places
Places SHALL show a "Docker" entry when a local Docker or Podman engine answers, and SHALL hide it when none does. Activating it SHALL open `docker:///` in the pane. Detection SHALL never block a frame.

#### Scenario: Engine starts later
- **WHEN** Docker Desktop is started after the app and Places is refreshed
- **THEN** the Docker entry appears without restarting the app
