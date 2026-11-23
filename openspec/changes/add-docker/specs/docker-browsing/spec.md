## ADDED Requirements

### Requirement: Local Docker engine discovery
The app SHALL look for a local Docker Engine API, in order: `DOCKER_HOST` when it is `unix://` or `npipe://`, the rootless socket in `$XDG_RUNTIME_DIR`, `/var/run/docker.sock`, `~/.docker/run/docker.sock`, the Podman socket in `$XDG_RUNTIME_DIR`, and `\\.\pipe\docker_engine` on Windows, checking each with `/_ping` off the UI thread. Other `DOCKER_HOST` schemes SHALL be ignored with a note in Preferences. A socket that exists but refuses this user SHALL produce a message explaining how to get access on that OS.

#### Scenario: Docker not running
- **WHEN** no engine answers at start
- **THEN** no Docker entry is shown and no error appears

#### Scenario: Permission denied
- **WHEN** `/var/run/docker.sock` exists but the user is not allowed to use it and the user opens `docker:///`
- **THEN** the pane says Docker is installed but this user may not use it, with the hint to join the docker group or use rootless Docker

### Requirement: Docker location
`docker:///` SHALL list `Containers`, `Images` and `Volumes`. Containers SHALL be listed with name, image, state (running, exited, …) and age; images with repository:tag (or short id), size and age; volumes with name and driver. These lists SHALL refresh with the normal refresh command.

#### Scenario: Containers folder
- **WHEN** the user opens `docker:///containers`
- **THEN** running and stopped containers are listed by name with image, state and age

### Requirement: Container file systems
Entering a container, running or stopped, SHALL show its file system. Files SHALL be readable, previewable and copyable out; copying files in, overwriting files and creating folders SHALL apply immediately; deleting and renaming SHALL be disabled with a tooltip giving the reason. The app SHALL NOT start, stop, remove or run commands in user containers.

#### Scenario: Edit a config file in a running container
- **WHEN** the user opens `/etc/nginx/nginx.conf` in container `web` with F4, edits and saves it
- **THEN** the file in the container has the new content and keeps its mode and owner

#### Scenario: Delete is disabled
- **WHEN** the cursor is on a file inside a container
- **THEN** Delete and Rename are disabled with a tooltip saying Docker cannot do this without running a command in the container

### Requirement: Image file systems
Entering an image SHALL show its merged file system read-only, without starting a container, by using a created-but-never-started helper container that is removed when the image is closed.

#### Scenario: Browse an image
- **WHEN** the user enters `docker:///images/alpine:3.20`
- **THEN** the pane lists `bin`, `etc`, `lib` and so on, write commands are disabled, and no container is running afterwards

### Requirement: Volume file systems
Entering a volume SHALL show its files through a created-but-never-started helper container with the volume mounted, made from an image already present locally. With no local image, the pane SHALL offer to pull `busybox` and SHALL NOT pull anything without that choice. Volumes SHALL follow the container rules for writing.

#### Scenario: No image present
- **WHEN** the engine has no images and the user enters a volume
- **THEN** the pane offers "Pull busybox to browse volumes" and nothing is pulled until the user chooses it

### Requirement: Helper containers are the app's own and are cleaned up
Helper containers SHALL carry the app's label and name prefix, have no network and no ports, never be started, and be removed when their image or volume is closed, on quit, and at start for any left over. Only containers carrying the app's label SHALL ever be removed.

#### Scenario: Leftover after a crash
- **WHEN** the app starts and the engine has a stopped container with the app's helper label from an earlier session
- **THEN** that container is removed and user containers are untouched

### Requirement: Docker and OCI image files open as archives
`docker save` tars and OCI image-layout tars SHALL open like archives, read-only, with all layers applied in order (whiteouts and opaque directories honoured) as one file system, plus a `Layers` folder listing each layer by position and short digest that can be browsed on its own. Gzip and zstd layer blobs SHALL be supported.

#### Scenario: Deleted file in a later layer
- **WHEN** layer 1 adds `/app/secret.txt` and layer 2 contains `/app/.wh.secret.txt`
- **THEN** the merged view has no `secret.txt`, and `Layers/1` still shows it

### Requirement: Docker commands
`docker.refresh`, `docker.pull_helper_image` and `docker.unmount` SHALL be commands with ids, menu paths and bad-parameter tests; names passed to them SHALL be validated and percent-encoded before any request.

#### Scenario: Hostile container name
- **WHEN** a command receives a container name containing `/../`, `?` or a newline
- **THEN** it returns `Err` or encodes the name so the request targets only that name
