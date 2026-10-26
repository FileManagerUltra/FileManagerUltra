## 1. Foundations

- [x] 1.1 `VfsRegistry` keyed by `(Scheme, authority)`; `get`/`require` use the uri; existing tests stay green
- [x] 1.2 Add `russh`, `russh-sftp`, `tokio` (fm-sftp only), `secrecy`, `zeroize`, `ssh-key` (via `russh::keys`, no separate dep); confirm `cargo xtask layers` and audit pass
- [x] 1.3 Secret types and the canary-secret redaction test across every `FmError` produced by connect paths

## 2. SFTP backend (`fm-sftp`)

- [x] 2.1 Shared runtime, `block_on` bridge, in-runtime call guard + test
- [x] 2.2 `read_dir`/`stat` with entry cap (error, like local reads), name validation; hostile-listing test
- [x] 2.3 `open_read`/`open_read_at`/`open_write` streaming in fixed chunks, idle timeout, cancel between chunks
- [x] 2.4 `create_dir`, `remove`, `rename`, `create_symlink`, `same_device`, `space` (statvfs extension, `Ok(None)` when absent)
- [x] 2.5 Error mapping to `FmError` (NotFound, PermissionDenied, AlreadyExists, Backend), redacted
- [x] 2.6 Test server: in-process russh SFTP server over a tempdir (password + key auth, fixed host key, `Faults` knobs) as `fm_fs_sftp::testserver` behind the `test-server` feature
- [x] 2.7 Round-trip, resume-offset, permission-denied and dropped-connection tests

## 3. Connections (engine)

- [x] 3.1 Connect job on the `sftp` lane; `remote.connect` with param validation and fuzz test (bad params → `Err`)
- [x] 3.2 Host-key check against owner-only `known_hosts.json`; TOFU confirm via `remote.hostkey.answer`; changed key refuses; tests incl. file mode
- [x] 3.3 Connection state machine (stale = connection not Ready; cached listings stay visible), shared "mutations disabled while not Ready" predicate; `remote.reconnect`, `remote.disconnect`
- [x] 3.4 Profiles in `prefs.toml` (cap, validation, no secret fields) + `remote.profile.save|delete`; round-trip test
- [x] 3.5 Visible-remote-pane poll (5 s, focus-regain refresh, hidden panes skipped)
- [x] 3.6 Space read job + cache (any backend; 30 s TTL)

## 4. Operations across backends

- [x] 4.1 The existing `.fmpart` temp write + rename-on-success covers remote destinations (verified by tests); cleanup on error and cancel
- [x] 4.2 Cross-backend move = copy then remove-after-success; same-authority move = rename
- [x] 4.3 Tests: local→SFTP, SFTP→local, SFTP→SFTP, cancel mid-transfer, failed copy keeps source

## 5. UI

- [x] 5.1 Connect dialog (host, port, user, auth, key path, start dir, profile picker, "kept for this session only" note)
- [x] 5.2 Host-key confirm dialog and changed-key error card
- [x] 5.3 Pane-chrome connection badge and stale styling (semantic tokens only; dark + light snapshots)
- [x] 5.4 Status-bar free-space field
- [x] 5.5 Menu entries (`File ▸ Connect to Server…`, Reconnect, Disconnect), command registry wiring, brand-check

## 6. Wrap-up

- [x] 6.1 Specs: security, file-operations deltas verified against tests; `openspec validate --all`
- [x] 6.2 `docs/architecture.md` (§4.5 registry key, host-key store), `docs/development.md` smoke checklist incl. a real OpenSSH server
- [x] 6.3 `cargo xtask ci` green; `log/devlog.md` entry
