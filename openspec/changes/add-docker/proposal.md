## Why

Developers and CTF players spend a lot of time inside containers: checking a config
file in a running service, pulling a log out of a stopped container, looking at what
an image ships, or fixing a file in a volume. Today that means `docker cp`,
`docker exec … cat` or `docker save | tar`, one file at a time. Browsing containers,
images and volumes like folders in a Files pane, with copy, preview, Find and edit,
makes that direct. Image files from `docker save` are archives too and should open
like them. Builds on `add-archive-vfs`.

## What Changes

- **A Docker location.** When a local Docker (or Podman with its Docker-compatible
  API) engine is reachable, a "Docker" entry appears in Places and the address bar
  accepts `docker:///`. It lists `Containers`, `Images` and `Volumes`.
- **Containers** (running and stopped) open as their file system. Files can be read,
  previewed and copied out; files can be copied in or overwritten and folders created,
  and these changes apply immediately like on SFTP, because a container is a live
  file system (no staging). Delete and rename are not possible through Docker's API
  without running a command in the container, which the app never does, so they are
  greyed out with that reason.
  Rows show name, image, state and age; Properties show the container's details.
- **Images** open read-only as their merged file system (all layers applied), without
  starting anything.
- **Volumes** open as folders with the same rules as containers, through a helper
  container that is created but never started, is labelled as the app's, and is removed when the volume
  is closed or at the next start.
- **Image files.** `docker save` and OCI image-layout tar files open like archives with
  their layers merged (whiteouts applied); a `Layers` folder shows each layer on its own.
- **No root needed beyond Docker access.** The app talks to the Docker Engine API over
  the local socket (Linux/macOS) or named pipe (Windows) with a small built-in HTTP
  client; it never runs the `docker` CLI and never starts or execs into containers.
- **Out of scope.** Remote engines (`tcp://`, `ssh://`, Docker contexts), Kubernetes,
  starting/stopping/removing containers, building images, registries, `docker exec`
  terminals, editing image content.
- Everything is a command (`docker.*`) with bad-parameter tests.

## Capabilities

### New Capabilities
- `docker-browsing`: engine discovery, the Docker location in Places, containers, images and volumes as folders, writing to containers and volumes, helper-container lifecycle, and Docker/OCI image files as archives.

### Modified Capabilities
- `places`: a Docker entry appears when a local engine is reachable.
- `security`: the Docker socket is root-equivalent; the app only uses the local engine, never starts, execs or removes user containers, labels and cleans up its own helpers, and treats engine responses and tar streams as hostile.

## Impact

- New crate `crates/fs-docker` (`fm-fs-docker`, L0): a small synchronous HTTP/1.1
  client over `std::os::unix::net::UnixStream` and Windows named pipes (`httparse`),
  the Engine API calls used here, and `DockerFs: Vfs`. No tokio, no `bollard`.
- `fm-vfs`: `Scheme::Docker` (`docker://<engine>/…`, empty engine = local default).
- `fm-archive`: Docker/OCI image-tar format (layer merge with whiteouts).
- `fm-engine`: backend registration, a `docker` job lane, discovery and helper cleanup
  at start, `docker_cmds.rs`.
- `fm-ui`: Places entry, container/image/volume rows and states, Properties details.
- `fm-brand`: the label key and helper-container name prefix.
- Dependency: `httparse` (MIT/Apache-2.0, pure Rust).
