## Why
Users expect "Network" in a file manager: hosts found on the LAN, shares under each host, and a credentials popup when a host asks for a login (Nemo, Explorer, Finder). Today SMB needs a typed host and share.

## What Changes
- **Share listing:** `smb://host/` lists the server's disk shares (srvsvc `NetrShareEnum`, classic NDR, which Samba/NAS accept), hiding `$` shares.
- **Discovery:** new crate `fm-discovery` (L0) finds hosts by mDNS/DNS-SD (`_smb._tcp`, `_sftp-ssh._tcp`) and WS-Discovery (Windows). Pure Rust, no daemon.
- **Network location:** `network:///` is a virtual folder listing discovered hosts ("nas (File Sharing)", "nas (Remote Login)"). It is the root of the tree in a Files pane, so hosts, shares and folders browse as one tree.
- **Authentication popup:** opening an unconnected `smb://` host (tree expand, double-click, typed address) shows a modal "Authentication Required" dialog (username, domain, password, remember choice) instead of the connect form. A remembered login connects silently next time.
- **Remember choices:** "Forget password immediately" and "Remember password until quit" (memory only). "Remember forever" is NOT offered: secrets never touch disk.
- `View ▸ Pane Type ▸ Network` and File ▸ Network open the Network location.

## Impact
- Code: `fm-vfs` (`Scheme::Network`), `fm-discovery` (new), `fm-fs-smb` (share listing, optional share), `fm-engine` (discovery job, login prompt, credential cache), `fm-ui` (dialog), `fm-app` (menu), `xtask layers`.
- Specs: new `network-browsing`; `smb-backend` and `remote-connections` deltas in `smb-backend` change adjusted (typed `smb://` opens the popup).
