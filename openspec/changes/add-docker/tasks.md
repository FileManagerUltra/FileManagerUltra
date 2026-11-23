## 1. Crate and spikes

- [ ] 1.1 Create `crates/fs-docker` (`fm-fs-docker`, L0), register in `xtask/src/layers.rs` and the workspace manifests (atomically); add `httparse` (exact pin); `cargo deny check` green
- [ ] 1.2 Spike: HTTP/1.1 over `UnixStream` and over `\\.\pipe\docker_engine` opened with `std::fs::OpenOptions` on Windows (via the Windows peer); `/_ping`, `/version`, archive `HEAD`/`GET`/`PUT`; note results in `design.md`
- [ ] 1.3 Recording fake engine for tests (in-process server on a temp Unix socket; on Windows a test-only named pipe or a transport trait with an in-memory fake)

## 2. Transport and API (`fm-fs-docker`)

- [ ] 2.1 HTTP client: request writer, `httparse` response parsing, Content-Length and chunked bodies, header/body caps, timeouts; hostile-response tests
- [ ] 2.2 Engine discovery (D2) with per-OS candidates and permission-denied detection
- [ ] 2.3 API calls: list containers/images/volumes, inspect, archive HEAD/GET/PUT, create/remove labelled helpers, pull image; JSON caps
- [ ] 2.4 `Scheme::Docker` in `fm-vfs` (`docker://<engine>/…`), name validation and percent-encoding; round-trip tests

## 3. `DockerFs: Vfs`

- [ ] 3.1 Root and the three lists with columns (name, image, state, age; size; driver)
- [ ] 3.2 Containers: first-level tar listing with caps, stat via HEAD, streaming read, write via temp + PUT (mode/owner kept), create_dir; remove/rename return `Unsupported` with the reason
- [ ] 3.3 Images via created-never-started helper containers (D5); read-only
- [ ] 3.4 Volumes via helper containers from a local image (D6); "Pull busybox" flow; write rules as containers
- [ ] 3.5 Helper lifecycle: labels and names from `fm-brand`, no network, removal on unmount/quit/start; tests with the fake engine that user containers are never touched

## 4. Image files (`fm-archive`)

- [ ] 4.1 `docker save` and OCI layout format: manifest/index parsing, layer order, gzip/zstd blobs, whiteouts and opaque dirs, merged index, `Layers` folder; fixtures and hostile tests

## 5. Engine and UI

- [ ] 5.1 `JobLane::Docker` and `BulkDocker`; backend registration; discovery at start and on Places refresh
- [ ] 5.2 `docker_cmds.rs`: `docker.refresh`, `docker.pull_helper_image`, `docker.unmount`; `commands.rs`; bad-param tests
- [ ] 5.3 Places "Docker" entry (shown when an engine answers); states for no access, no image, engine gone
- [ ] 5.4 Disabled Delete/Rename with reason tooltip inside containers and volumes; read-only images
- [ ] 5.5 egui_kittest snapshots (dark and light): Docker root, containers list, inside a container, no-access state

## 6. Docs, checks and wrap-up

- [ ] 6.1 `docs/architecture.md` (Docker backend), `docs/roadmap.md`, AGENTS.md crate list
- [ ] 6.2 `CHANGELOG.md` entries
- [ ] 6.3 `cargo test`, clippy `-D warnings`, `cargo xtask layers`, `cargo xtask brand-check`, `cargo fmt --check`, `cargo deny check`
- [ ] 6.4 Manual smoke: Linux (rootful and rootless Docker, Podman), Windows (Docker Desktop), macOS (Docker Desktop)
- [ ] 6.5 `log/devlog.md` entry
