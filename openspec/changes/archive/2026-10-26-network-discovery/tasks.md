## 1. Share listing
- [x] 1.1 `NetrShareEnum` over srvsvc with classic NDR (the crate's NDR64 bind is refused by Samba); parser unit-tested against hostile replies
- [x] 1.2 `smb://host/` lists server disk shares; sign-in without a share; Samba test

## 2. Discovery
- [x] 2.1 `Scheme::Network` and `network:///` URI (parse/display/round-trip tests)
- [x] 2.2 `fm-discovery` crate: host model, caps/sanitising, `NetworkFs`, layer registration
- [x] 2.3 mDNS browse (`_smb._tcp`, `_sftp-ssh._tcp`)
- [x] 2.4 WS-Discovery probe and `XAddrs` parser (hostile-input tests)

## 3. Engine
- [x] 3.1 Discovery started on first `network:///` read; host list refreshes the listing
- [x] 3.2 Login prompt state for unconnected `smb://` reads; `network.login`/`network.login.cancel` commands
- [x] 3.3 In-memory credential cache with the two remember modes; silent connect when cached
- [x] 3.4 Unconnected `sftp://` read opens the SFTP form
- [x] 3.5 Bad-params tests for the new commands

## 4. UI
- [x] 4.1 Pane type `Network` enabled; File ▸ Network; tree root `Network`
- [x] 4.2 Authentication Required popup (username, domain, password, remember choice; Enter connects, Esc cancels)
- [x] 4.3 Snapshots: popup and Network listing, dark and light

## 5. Wrap-up
- [x] 5.1 Docs (architecture, development smoke checklist), devlog
- [x] 5.2 `cargo xtask ci`, `openspec validate --all`, audit of `mdns-sd` tree
