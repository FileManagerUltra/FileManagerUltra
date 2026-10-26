## Context

`fm-sftp` is a 73-line stub. The `Vfs` trait, `VfsUri` (with authority), the `sftp` job lane, `ops` streaming copy and the pane code are already backend-agnostic, so this change is mostly: a real backend, a connection manager, and the UI to drive it. Architecture §4.5 already fixes the user-facing flow; this document records the decisions that §4.5 leaves open.

## Goals / Non-Goals

**Goals:** SFTP as a first-class pane source with local↔remote and remote↔remote ops; safe host-key and credential handling; visible connection state; profiles without secrets; free space in the status bar.

**Non-Goals:** SMB (own change); saving passwords (future, OS keyring); SSH agent, jump hosts, port forwarding; remote recursive folder-size scans for the preview; remote thumbnails/video playback beyond what `open_read` already gives.

## Decisions

1. **Registry keyed by `(Scheme, authority)`.** `VfsRegistry` currently holds one backend per scheme. Change the map key to `(Scheme, String)` with `""` for local; `get`/`require` take the `VfsUri`. Callers that pass a bare scheme (`lane_for`, tests) keep working. *Alternative:* one `SftpFs` multiplexing many servers, which hides per-connection state and makes "disconnect one server" awkward.

2. **One `SftpFs` per connection, owning an `Arc<Runtime>` shared by the crate.** A process-wide lazily created multi-thread runtime (2 workers) is shared; each `SftpFs` holds its `SftpSession`. Every `Vfs` method does `runtime.block_on(...)` from a lane worker. A guard (`Handle::try_current()` fails → ok, succeeds → error) turns accidental calls from inside the runtime into `FmError`, not a deadlock. *Alternative:* a runtime per connection: more threads for no gain.

3. **Connect is a job.** `remote.connect` submits a `connect` job on the `sftp` lane; the handshake, auth and SFTP subsystem open run there. Host-key confirmation needs the user, so the job parks on a channel exactly like the conflict dialog (`on_conflict` pattern in `fileops`): the engine exposes `pending_hostkey`, the UI answers with `remote.hostkey.answer {accept}`. Cancel closes the channel and the job ends.

4. **Host keys.** `russh`'s `check_server_key` runs in the handler; it compares against `known_hosts.json` in the state dir (owner-only, written atomically, max 1000 entries, 4 KiB per line). Unknown → ask (decision 3). Different → return an error variant carrying both fingerprints (SHA-256, base64) and refuse. No "accept new key" path in v1: the user edits/removes the entry deliberately.

5. **Secrets.** `secrecy::SecretString` for password/passphrase, kept in a `Credentials` value in the session's connection table (not in `Prefs`, not serialized). `FmError` never embeds them; the connect error mapper builds messages from fixed strings plus host/user only. A test formats every error path with a canary secret. Reconnect reuses the held credentials; after a failed auth they are dropped and the dialog asks again.

6. **Connection state machine** per authority in `Session`: `Connecting | Ready | Failed(String)`; transitions only from connect/disconnect jobs and from I/O errors classified as "transport" (broken pipe, EOF, timeout). `Failed` marks the authority's cached directories stale and makes `enabled` false for mutating commands whose focused pane is on that authority (one predicate helper reused by every file command, not per-command checks).

7. **Refresh without watcher.** A once-per-second engine tick looks at visible remote panes; if a pane has been Ready and unread for 5 s (and no read in flight) it calls `refresh_read`. Hidden panes (other tab, collapsed) are skipped. Focus regain triggers one immediate refresh. Reads go through `DirCache`, so two panes on one directory share the poll.

8. **Cross-backend ops.** `ops` already streams. Changes: (a) move across different backends = copy then remove-after-success; (b) writes go to `name.fm-part` then `rename` on success so a cancel/failure never leaves a partial file, and the temp file is removed on error (local keeps its current behaviour); (c) `same_device` is true only for the same authority, so same-server moves use `rename`.

9. **Profiles** are `[[profiles]]` in `prefs.toml` (name, host, port, user, key_path, start_dir), capped at 50, field lengths validated on load. `Prefs` keeps "no secret fields"; a test asserts the serialized form of a profile contains no password key.

10. **Free space.** `Vfs::space` already exists. A `space` read job (per location, cached for 30 s, on the location's lane) feeds a status-bar field; SFTP uses the `statvfs@openssh.com` extension and returns `Ok(None)` when absent.

11. **Testing without a network.** `russh` and `russh-sftp` provide server building blocks: the test crate runs an in-process SFTP server on `127.0.0.1:0` over a temp directory (password and key auth, a chosen host key) plus a fault-injecting wrapper (drop connection, slow reads, hostile listing). All SFTP tests run in CI offline. A manual smoke checklist covers a real OpenSSH server.

## Risks / Trade-offs

- **Blocking over async** → deadlock if misused. Mitigation: decision 2 guard plus a test that calls from the runtime.
- **Hostile servers** (huge/absurd listings, lying sizes, never-ending reads) → caps before allocation, per-call timeout (30 s idle), cancel flag checked between chunks.
- **Reconnect races** (job in flight when the link drops) → jobs hold their own `Arc<SftpFs>`; a dead session returns a transport error, which fails that job and flips state once (idempotent).
- **Credential leakage** → centralised error mapping plus the canary test; no `Debug` on secret types.
- **Dependency weight** (`russh`, `ring`/`aws-lc`, `tokio`) → feature-trimmed, isolated to `fm-sftp`; check `cargo xtask layers` still passes (no new crate).
- **Poll cost** → 5 s per visible remote pane, shared via cache; configurable later.

## Migration Plan

Additive. `Scheme::Sftp` URIs already parse. The registry key change is internal; local-only sessions behave as before and all existing tests must stay green. No data migration: `prefs.toml` gains an optional `profiles` list; older files load unchanged.

## Open Questions

- Key formats: OpenSSH (ed25519/RSA) required; PuTTY `.ppk` deferred?
- Default for a changed host key: strictly refuse (proposed) vs. allow the user to replace via a visible "Forget host key" command.
- Should profiles also be selectable from a location-bar `sftp://` typed address (prompting for the password), in this change or later?
