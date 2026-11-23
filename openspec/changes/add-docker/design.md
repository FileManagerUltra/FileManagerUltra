## Context

Backends implement `Vfs` and are registered per `(scheme, authority)`; SFTP and SMB
show how a connection-based backend with its own lane fits. `add-archive-vfs`
provides archive formats, the archive look and read-only handling; this change adds a
live Docker backend and one more archive format (image tars).

The Docker Engine API (v1.41+, also served by Podman's compatible socket) exposes a
container's file system through the archive endpoints, which work for running and
stopped containers:

- `HEAD /containers/{id}/archive?path=…` → stat in the `X-Docker-Container-Path-Stat`
  header (base64 JSON: name, size, mode, mtime, linkTarget);
- `GET /containers/{id}/archive?path=…` → a tar stream of that path;
- `PUT /containers/{id}/archive?path=<dir>` → extract an uploaded tar into `dir`.

There is no file API for images or volumes, and no delete or rename endpoint.

Constraints: pure Rust, no `unsafe`, no FFI; `tokio` stays private to `fm-fs-sftp`
(AGENTS.md §7), so no `bollard`; UI never blocks; hostile input; everything a command.

User decision (2026-11-23): Docker is its own change after archives.

## Goals / Non-Goals

**Goals:** browse and edit container file systems; browse images read-only; browse and
edit volumes; open `docker save`/OCI tars; never disturb the user's containers.

**Non-Goals:** remote engines, lifecycle management (start/stop/rm), exec, builds,
registries, Kubernetes.

## Decisions

### D1. Transport: a small synchronous HTTP client

`fm-fs-docker` writes HTTP/1.1 requests over `UnixStream` (Linux, macOS) or a named
pipe opened with `std::fs::OpenOptions` (Windows, `\\.\pipe\docker_engine`), and parses
responses with `httparse` (status line, headers, `Content-Length` and chunked bodies,
capped header size and count). One request per connection keeps it simple; listings are
small and file bodies stream. Timeouts: 5 s connect, 30 s idle per read.
Spike 1.2 confirms Windows named pipes work with plain `std` file I/O for the Docker
pipe; if not, Windows support moves to a follow-up.

### D2. Engine discovery

In order: `DOCKER_HOST` when it is `unix://` or `npipe://` (other schemes are ignored
with a note in Preferences); `$XDG_RUNTIME_DIR/docker.sock` (rootless);
`/var/run/docker.sock`; `~/.docker/run/docker.sock` (Docker Desktop on macOS and
Linux); `$XDG_RUNTIME_DIR/podman/podman.sock`; `\\.\pipe\docker_engine` (Windows).
`GET /_ping` and `GET /version` decide availability and API version. Discovery runs
at start and when Places is refreshed, off the UI thread; permission denied on the
socket shows "Docker is installed but this user may not use it" with the per-OS hint
(docker group, rootless, Docker Desktop running).

### D3. Locations

`Scheme::Docker`, `docker://<engine>/…`, engine empty for the discovered local one.
Paths: `/containers/<name>/<path>`, `/images/<repo:tag or id>/<path>`,
`/volumes/<name>/<path>`. Names are untrusted and percent-encoded in requests; a
container's id is resolved from its name on every mount so renames are noticed.

### D4. Containers

- `read_dir`: `GET …/archive?path=<dir>` streams a tar; the reader keeps only the
  first level (entries below are skipped without buffering their data, bounded by an
  entry cap). Stat via `HEAD`.
- `open_read`: `GET …/archive?path=<file>` and stream the single member's data.
- `open_write`: buffer to the private temp folder, then `PUT` a one-file tar into the
  parent directory on close (mode and owner preserved for replaced files).
- `create_dir`: `PUT` a tar with one directory entry.
- `remove` and `rename`: the API has no endpoint for them, and the app never runs
  commands inside user containers, so they are **not supported** in this change.
  They are greyed out with the reason ("Docker cannot delete or rename files inside a
  container without running a command in it"). Copy in, overwrite and New folder work.
- Writes apply immediately, like SFTP: a container is a live system, not a file to save.

### D5. Images

Images are mounted read-only by creating (never starting) a container from the image
with the app's label (D7), browsing it as in D4, and removing it on unmount or idle.
This avoids exporting gigabytes with `docker save`.

### D6. Volumes

A volume is mounted by creating (never starting) a helper container with the volume at
`/volume`, from an image already present locally (preferring small ones: `busybox`,
`alpine`, then any). If no image is present, the pane offers "Pull busybox to browse
volumes" (explicit; never automatic). Volumes follow the container rules: read, copy
in, overwrite and New folder; no delete or rename. A follow-up can add delete and
rename through an opt-in helper that *starts* a throwaway container with only that
volume mounted and runs a fixed `rm`/`mv` argument array in it, never touching user
containers.

### D7. Helper containers

Created with `Labels: { "<brand-id>.helper": "1" }`, name `<brand-id>-helper-<random>`
(from `fm-brand`), `HostConfig.AutoRemove=false`, no ports, no network
(`NetworkMode: none`), never started. Removed (`DELETE /containers/{id}`) on unmount,
on quit, and at start for any leftovers carrying the label. Only containers with this
label are ever removed.

### D8. Image files as archives

A new `fm-archive` format reads `docker save` (`manifest.json` + layer tars) and OCI
image layouts (`index.json`, `blobs/sha256/…`) in a tar: layers are applied in order,
with `.wh.<name>` whiteouts and `.wh..wh..opq` opaque directories, into one merged
read-only index. A `Layers` folder at the root lists each layer by index and short
digest, each browsable on its own. Layer blobs may be gzip or zstd compressed.

### D9. Lanes

`JobLane::Docker` (2 workers) serves listings and reads; bulk copies use `BulkDocker`.

## Risks / Trade-offs

- [No delete/rename in containers and volumes] → clearly greyed out with the reason;
  follow-up can add an opt-in volume helper. Keeps the promise of never running
  commands in user containers.
- [Listing a large directory streams its whole subtree as tar] → first-level filter
  with an entry cap and a byte cap; huge directories (e.g. `/usr`) list slower than on
  disk, and the pane shows progress while they load.
- [Socket access is root-equivalent] → only the local engine, fixed endpoints, no
  start/exec/rm of user containers, labelled helpers; security spec.
- [Podman compatibility] → tested in CI if a Podman socket is available, otherwise
  manual smoke.

## Migration Plan

Additive.

## Open Questions

- Should the Docker entry live in Places (proposed) or under Network? Proposed: Places,
  shown only when an engine answers.
- Do we want the opt-in volume helper for delete and rename in this change?
