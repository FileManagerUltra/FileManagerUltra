## Context

`search_cmds::start` allows `Scheme::File | Scheme::Sftp` and runs either the multi-threaded `LocalWalk` (native local) or the single-threaded generic `Walker` on an `fm-search` thread. `props_cmds::start` filters its walk to the same two schemes. Both were written under the AGENTS.md rule "SMB work stays on the `smb` lane". The SMB backend (`fs-smb`) opens one handle per `read_dir`/`stat`/`open_read`, closes it when done (`SmbReader` closes on drop), and is already used by two lane workers at once. Every SMB call is bounded by the configured timeout.

## Goals / Non-Goals

**Goals:**
- Find Files and Properties sizes work on SMB shares with the existing generic walker and caps.
- Clear messages where there is nothing to search.

**Non-Goals:**
- Parallel walking on SMB (one walker thread, as for SFTP).
- Searching an SMB server root across all its shares (it lists only shares connected so far).
- Reporting a lost connection from the walk to the connection banner (SFTP walks do not either; the next directory read on the lane does).

## Decisions

1. **Allow `Scheme::Smb` when the uri names a share** (`uri.share().is_some()`); route it to the generic `Walker`. Rationale: same code path as SFTP, already tested for caps, cancel and unreadable folders (skipped).
2. **One rejection helper** `searchable(uri) -> Result<(), FmError>` in `search_cmds`, used by both search and Properties, returning `FmError::Unsupported` with the message for Network/Servers ("Open a share or server to search it") and an SMB server root ("Open a share to search it").
3. **Properties** keeps its existing note for locations it cannot walk; the filter becomes `searchable(uri).is_ok()`.
4. **Docs:** AGENTS.md §7 and architecture §7 say directory reads and file jobs stay on the `smb` lane, and bounded single-thread walks (Find Files, Properties sizes) may use the session from their own thread.

## Risks / Trade-offs

- [A walk plus two lane workers share one SMB session] → One extra sequential reader; handles are per operation and closed. Content search reads at most 64 MiB per file and stops on Stop.
- [A slow share makes a search slow] → Same as SFTP; the window shows progress and Stop cancels between calls (a blocked call ends at the SMB timeout).
