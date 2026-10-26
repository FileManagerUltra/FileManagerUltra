## Why

Every pane can browse a local path, but the product promise is "any pane can browse a remote". `fm-sftp` is a stub that returns `Unsupported`, so copying to or from a server still means leaving the app. SFTP is the most requested remote and the simplest to get right (pure Rust, no system library), so it lands first; SMB follows as its own change.

## What Changes

- **`fm-sftp` becomes a real `Vfs`** (`russh` + `russh-sftp`, one tokio runtime owned by the crate): listing, stat, streamed read/write (with resume offset), create dir, remove, rename, symlinks, free space.
- **Connect flow**: `remote.connect` opens a dialog (host, port, user, auth: password | private key + optional passphrase, start dir). The handshake runs on the `sftp` lane; success registers the backend under its authority and navigates the focused pane to `sftp://user@host:port/dir`.
- **Host keys**: trust-on-first-use with an explicit fingerprint confirm; a changed key is a hard refusal with an actionable message. Known hosts persist in an owner-only file.
- **Connection state** per authority (`Connecting → Ready → Failed`), shown as a badge in the pane chrome. On loss the last listing stays visible and marked stale; mutating commands are disabled until `remote.reconnect`.
- **Profiles**: named connections (host, port, user, key path, start dir) persist in `prefs.toml`. **Secrets are never saved** (passwords and passphrases are memory-only). Saving credentials is an explicit non-goal now, to be designed later (OS keyring).
- **Free space** for the focused location in the status bar (`Vfs::space`), local and SFTP.
- **Refresh without `notify`**: remote panes refresh on focus regain, `F5`, and a 5 s poll while visible.
- **Registry per authority**: `VfsRegistry` today maps one backend per scheme; it becomes `(scheme, authority)` so two servers can be open at once. Local behaviour is unchanged.
- Out of scope: SMB (next change), saving credentials, SSH agent/jump hosts, SFTP-side search, remote recursive folder scans for the preview.

## Capabilities

### New Capabilities
- `sftp-backend`: the `Vfs` implementation over SFTP, its limits, error mapping and lane behaviour.
- `remote-connections`: connect dialog and commands, connection state, host-key trust, profiles, stale/reconnect behaviour, remote refresh, free-space display.

### Modified Capabilities
- `security`: credentials memory-only and redacted for SFTP, host-key trust, remote listing/size caps, owner-only known-hosts file.
- `file-operations`: cross-backend copy/move (local ↔ SFTP, SFTP ↔ SFTP), same-authority rename-as-move, partial files never left behind on failure/cancel.

## Impact

- **Code:** `fm-sftp` (new implementation), `fm-vfs` (authority-aware lookup types if needed), `fm-engine` (`remote_cmds.rs`, `VfsRegistry`, sftp lane connect job, known-hosts/profile store, poll refresh), `fm-ui` (connect dialog, host-key confirm, state badge, status-bar space), `fm-app` (menu, wiring), `fm-testkit` (in-process SFTP test server or fault-injecting remote), `xtask layers` (no new crate).
- **Dependencies:** `russh`, `russh-sftp`, `tokio` (inside `fm-sftp` only), `secrecy` + `zeroize`, `ssh-key` (fingerprints) as needed. `cargo deny`/audit review for each.
- **Risks:** blocking-trait-over-async bridging (deadlocks if a `Vfs` call runs on the runtime thread); hostile servers (huge listings, lying sizes, slow-loris); credential leakage via error strings or logs; reconnect state races with in-flight jobs.
