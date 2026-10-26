## Why

SFTP covers servers; SMB covers the NAS and Windows shares most people actually have. `fm-smb` is a stub that returns `Unsupported`. The original plan (pavao/libsmbclient on Linux/macOS, native UNC on Windows) needs a system library and two code paths. The pure-Rust `smb` crate (SMB 2/3 client, NTLM) gives one code path on every platform with no system library, which fits the "light, simple, cross-platform" goal. It reuses the connection model built for SFTP.

## What Changes

- **`fm-smb` becomes a real `Vfs`** on the pure-Rust `smb` crate (sync mode, no tokio in this crate): listing, stat, streamed read/write (resume offset), create dir, remove, rename, free space. URIs are `smb://[user@]host[:port]/share/path`; the first path segment is the share.
- **SMB pane type** (`PaneKind::Smb`, a files kind like SFTP) with an inline connect form: host, port, user, optional domain, password, share. The same Disconnect button, stale banner, profile picker, offline/reconnect behaviour and poll refresh as SFTP.
- **Auth: NTLM username/password only.** Guest/anonymous when the user is left empty. No Kerberos, no saving passwords (memory only, as for SFTP).
- **Signing and encryption** follow what the server negotiates; unsigned/unencrypted fallback is never silent: the connection badge tooltip states "signed", "encrypted" or "NOT signed".
- **Share enumeration** is out of scope for v1; the user types the share (the connect form remembers it in the profile). A bare `smb://host/` shows a hint instead of a listing.
- **Concurrency**: the `smb` lane is no longer a single thread (that limit existed only for pavao's global lock); it gets the same 2 workers as `sftp`.
- **Docs**: `AGENTS.md`, `docs/architecture.md`, `docs/development.md` and the roadmap drop pavao and the native-UNC path.
- Out of scope: Kerberos/SSO, share browsing, DFS, SMB over QUIC/NetBIOS, saving credentials, Windows "mapped drive" shortcuts.

## Capabilities

### New Capabilities
- `smb-backend`: the `Vfs` over SMB, bounds, error mapping, signing/encryption reporting.

### Modified Capabilities
- `remote-connections`: SMB profiles (domain, share), no host-key step, connect form fields, status of signing/encryption.
- `pane-types`: SMB pane kind.
- `security`: SMB credentials memory-only and redacted, NTLM only, signing state visible, remote caps apply.

## Impact

- **Code:** `fm-smb` (implementation, `test-server`-free: tests use Samba in Docker), `fm-layout` (`PaneKind::Smb`), `fm-engine` (`remote.rs` generalised from "sftp" to a protocol, SMB connect job, `smb` lane workers), `fm-ui` (connect form variant, badge text), `fm-app` (menu entries), `xtask layers` (no new crate).
- **Dependencies:** `smb` with default features off (`sign`, `encrypt`, `single_threaded`/sync; no `quic`, `rdma`, `kerberos`). It pulls `sspi` unconditionally, so measure `cargo tree` before/after and run audit/deny; if the footprint is unacceptable, stop and revisit before writing more code.
- **Risks:** `smb` is 0.x (API churn, protocol edge cases) so it is pinned exactly and wrapped in a thin adapter; interop varies by server (Samba, Windows, NAS firmware), so tests run against Samba and the manual smoke lists Windows/NAS; NTLM-only fails on servers that require Kerberos (clear error).
