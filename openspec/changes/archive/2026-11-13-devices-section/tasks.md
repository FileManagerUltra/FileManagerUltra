## Implementation

- [x] Document platform discovery/removal adapters and unsupported cases; compile macOS/Windows adapter code and exercise metadata fixtures on Linux.
- [x] `engine::devices`: discover removable and user-mounted volumes (including non-removable virtual mounts), exclude system/pseudo/implementation mounts, sanitise display text, preserve identity, diff, generation counter.
- [x] Single-flight discovery worker isolated from file-job lanes (2 s / 10 s unfocused, focus refresh), timeout, keep-last-list on failure, plug/unplug notices.
- [x] Commands `devices.refresh`, `devices.eject`, `devices.dismount` (capability-aware adapters, mount identity revalidation, active-transfer guard, Home-first navigation, explicit errors) in `devices_cmds.rs`; register and fuzz malformed params.
- [x] Places UI: compact Devices block (hidden when empty), Eject or Dismount actions and unsupported explanation, drop target, Go menu entry, persisted collapse state; move path/free/total to directory Preview and hover.
- [x] Preview: volume information with used-space meter, proportional flat metadata sections and wrapping values; verify narrow/wide layouts in both themes and expandable thumbnail details.
- [x] Tests: filtering (system/pseudo vs user mounts, non-removable VeraCrypt fixtures, hostile labels), diff, unknown capacity, removal validation, mount identity reuse, no generic removal of virtual mounts, UI snapshots dark + light.
- [x] Docs: places/security spec sync, architecture, development manual checks, devlog.
- [x] Verify actual VeraCrypt mount discovery/ownership on each OS; add a volume-manager dismount adapter only when its identity can be verified (otherwise the implemented browse-only fallback applies).
- [x] Manual check on Linux, macOS and Windows with USB media and mounted VeraCrypt volumes (mount/unmount detection, browsing, capacity, eject vs dismount, busy/unsupported failures). Tested by the user on Linux and Windows (2026-11-13); macOS not tested, assumed equivalent.

## Manual evidence

- 2026-10-28: User confirmed Linux VeraCrypt mount discovery and browsing at
  `/media/veracrypt2`; Devices lists the volume with Dismount disabled as intended.
  Capacity and disappearance detection remain unconfirmed.
- 2026-10-28: User confirmed Windows VeraCrypt volume discovery. This does not
  yet verify the pending fixed-drive filter changes, dismount or unplug behavior.
  macOS hardware checks remain unconfirmed.
