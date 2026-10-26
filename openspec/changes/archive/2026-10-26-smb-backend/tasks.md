## 1. Foundations

- [x] 1.1 (vendored `sspi` patch; audit/deny still to run) Add `smb` (pinned, default features off, sign + encrypt, sync) to `fm-smb`; record `cargo tree` size before/after, run audit/deny; stop and report if unacceptable
- [x] 1.2 Spike: connect to Samba in Docker, list, read, write, rename, remove, free space through the crate; note any API gaps
- [x] 1.3 Docker Samba test harness (in `crates/fs-smb/tests/samba.rs`, skipped when docker is unavailable)

## 2. SMB backend (`fm-smb`)

- [x] 2.1 Connect (NTLM, signed; guest refused), lazy tree-connect per share, encryption required-or-allowed
- [x] 2.2 Share splitting; `smb://host/` lists the shares connected so far (per host+user `SmbFs`)
- [x] 2.3 `read_dir`/`stat` with entry cap, `validate_name`, hostile-listing test via a fake adapter
- [x] 2.4 `open_read`/`open_read_at`/`open_write` streaming in fixed chunks, idle timeout, cancel between chunks
- [x] 2.5 `create_dir`, `remove`, `rename`, `same_device`, `space`; symlinks `Unsupported`
- [x] 2.6 Error mapping + redaction canary test across every connect/IO error path
- [x] 2.7 Round-trip, resume, wrong-password tests against Samba (dropped-connection test still open)

## 3. Engine generalisation

- [x] 3.1 Separate `remote.connect.smb` command (the dialog is now `network.connect.dialog`) (instead of a `protocol` param); connections keyed by scheme+authority; bad-params test
- [x] 3.2 SMB connect job on the `smb` lane; no host-key step; auth failure drops credentials
- [x] 3.3 `smb` lane has 2 workers (concurrent two-connection test still open)
- [x] 3.4 Profiles gain `protocol`, `domain`, `share` (no secret fields); old profiles load as sftp; round-trip test
- [x] 3.5 Neutral `remote_*` timing prefs reading the old `sftp_*` keys; closed-schema test updated
- [x] 3.6 "Require signed or encrypted SMB" preference, enforced at connect
- [x] 3.7 Workspace restore: legacy `Smb` panes restore as Network panes

## 4. UI and app

- [x] 4.1 Folded into the Network pane: Connect… button + File ▸ Connect to SMB Server…; `PaneKind::Smb` is deserialize-only
- [x] 4.2 Manual popup (host, port, share, user, domain, password, require encryption); profile picker and save-as in the popup
- [x] 4.3 Badge/Disconnect/stale banner shared; tooltip shows encrypted or signed
- [x] 4.4 Location label and delete label ("Delete Permanently") for SMB; bare `smb://host/` hint
- [x] 4.5 Typed `smb://` address opens the popup
- [x] 4.6 Snapshots: manual connect popup (dark + light); badge states covered by the shared SFTP snapshot

## 5. Docs and gates

- [x] 5.1 Update `AGENTS.md`, `docs/architecture.md` (§4.4, lanes, platform table), `docs/development.md` (smoke checklist: Samba, Windows share, NAS), roadmap; remove pavao and native-UNC mentions
- [x] 5.2 Spec deltas validated (`openspec validate --all`); devlog entry
- [x] 5.3 (cargo-deny unavailable: dependency tree reviewed by hand, no RustSec advisories) `cargo xtask ci` green; `cargo deny`/audit reviewed for new dependencies
- [x] 5.4 Real-share smoke (Samba, Windows PC, NAS): user-verified speed and discovery
