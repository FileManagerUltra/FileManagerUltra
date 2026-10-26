## Context

Change 4 made remote access backend-agnostic: registry keyed by `(scheme, authority)`, per-authority connection state, connect job + stale/reconnect behaviour, poll refresh, profiles, cross-backend ops with `.fm-part` temp files, `Vfs::space`. SMB should be mostly an adapter plus a form variant. `Scheme::Smb`, `JobLane::Smb` and `VfsUri` parsing for `smb://` already exist.

## Goals / Non-Goals

**Goals:** SMB shares as first-class pane sources with local↔SMB, SMB↔SFTP and SMB↔SMB ops; one code path on Linux, macOS and Windows; no system library; credentials never persisted.

**Non-Goals:** Kerberos, share browsing, DFS, QUIC/NetBIOS transports, saving passwords, Windows UNC shortcuts, server-side copy.

## Decisions

1. **Pure-Rust `smb` crate, sync mode, features trimmed.** Default features off; enable only signing and encryption (plus whatever sync mode needs). No tokio in `fm-smb`: the crate's sync client runs on the `smb` lane workers directly, so the "no other crate depends on tokio" rule holds. *Alternative considered:* pavao/libsmbclient (system lib, serialises all calls, Windows needs a different path); rejected.
2. **Thin adapter, pinned version.** All `smb` types stay inside `fm-smb/src/client.rs`; the rest of the crate speaks `fm-vfs`. The dependency is pinned exactly (`=0.12.x`) so an upgrade is a deliberate change with the test suite as the gate.
3. **URI model.** `smb://[user@]host[:port]/share/path...`. Authority is `user@host[:port]` exactly like SFTP, so the registry, connection table and profile code are reused. The first path segment is the share; an operation on `/` (no share) returns a typed error with a hint. One connection per authority holds a tree-connect per share, opened lazily and kept.
4. **Domain.** Optional, stored in `Credentials` and the profile (not secret), passed to NTLM; not part of the authority. Two logins to one host with different domains but the same user name share an authority; acceptable for v1 and documented.
5. **Remote layer generalised, not duplicated.** `remote.rs` keys its model by authority and currently assumes SFTP. Introduce a small `Protocol` enum (`Sftp`, `Smb`) on `ConnectParams`/the connection record; protocol-specific parts are only: the connect function, whether there is a host-key step, and the form fields. Commands become `remote.connect {protocol, ...}` with `protocol` defaulting to `sftp` so existing callers and saved workspaces keep working.
6. **Connect job on the `smb` lane**, with a handshake/auth timeout from the existing `sftp_timeout_secs`-style prefs; the setting is renamed to a neutral `remote_*` key with the old keys still read (migration: read old, write new). Auth failure drops credentials as for SFTP.
7. **Signing/encryption report.** After negotiation the backend exposes `Security { signed, encrypted }`; the badge tooltip shows it. If neither is active the tooltip says so; a preference "Require signed or encrypted SMB" (default off, so NAS devices with old firmware still work) refuses unsigned connections.
8. **Error mapping.** `STATUS_OBJECT_NAME_NOT_FOUND`/`PATH_NOT_FOUND` → `NotFound`, `ACCESS_DENIED` → `PermissionDenied`, `OBJECT_NAME_COLLISION` → `AlreadyExists`, `LOGON_FAILURE` → an auth error with a fixed message, `BAD_NETWORK_NAME` → "share not found", connection reset/timeout → transport error (link lost, same classifier as SFTP). Messages are built from fixed strings plus host/share/path, never from library `Display` text that might echo credentials.
9. **Bounds and hostile servers.** Same rules as SFTP: entry cap enforced while reading directory chunks, `validate_name` on every entry, sizes never used to allocate, fixed-size read/write chunks, cancel flag checked between chunks, per-call idle timeout.
10. **Operations.** `rename` within one share maps to SMB rename (`SetInfo`); across shares/hosts ops fall back to copy+remove. `same_device` is true for same authority and same share. Remote write uses `name.fm-part` then rename, as for SFTP. Symlinks are not created (`Unsupported`); reparse points are listed as files/dirs without following them.
11. **Free space** via the share's filesystem-size query (`FileFsSizeInformation`/full-size); `Ok(None)` if the server refuses.
12. **Testing.** Integration tests run against Samba in Docker (`dperson/samba`-style image, user/password share), started by the test harness when `docker` is available and skipped otherwise, like the real-trash test. Unit tests cover URI/share splitting, error mapping, redaction canary, and hostile-listing handling via a fake adapter trait. Manual smoke adds a Windows share and one NAS.

## Risks / Trade-offs

- **Young crate** → pin, adapter boundary, Docker interop tests; if a blocking defect appears, the adapter is the only code to replace.
- **Dependency weight** (`sspi` pulled in regardless of features) → measure first (task 1.1); abort criteria stated in the proposal.
- **NTLM only** → servers that disable NTLM fail; the error says Kerberos is not supported yet.
- **Docker tests flaky/slow** → one shared container per test run, skipped when unavailable, never required for `cargo xtask ci` to pass.
- **Lane concurrency raise (1 → 2)** → safe now that no global lock exists; verify with a two-share concurrent test.

## Migration Plan

Additive. `Scheme::Smb` URIs already parse; stored workspaces with `smb://` locations restore to the connect form like SFTP. `remote.connect` without `protocol` behaves as before. Old `sftp_*` timing prefs keep loading. Docs drop pavao and the native-UNC path.

## Open Questions

- Share listing (`srvsvc` via `smb-rpc`) as a follow-up once the base works?
- Should "Require signed or encrypted" default to on for non-RFC1918 hosts?
