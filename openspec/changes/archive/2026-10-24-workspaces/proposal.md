## Why

Real work needs several pane arrangements (a dev setup, a media setup, a backup setup) that can be switched instantly and brought back after a restart. Today there is one layout, and nothing persists across runs.

## What Changes

- **Workspace tabs**: a tab bar holds any number of open workspaces; each owns a full pane tree with all per-pane state (locations, sort, selection, previews, terminals, thumbnails, zoom). Background tabs stay alive.
- **Saved workspaces**: up to 10 named workspaces persist in `workspaces.json` (owner-only, no secrets). Per-workspace flags: Auto-save, Open at startup, Default.
- **Startup**: workspaces flagged Open at startup open; otherwise the Default one; otherwise a built-in single pane.
- **Exit prompt**: non-auto-save workspaces with unsaved changes (and never-saved user tabs) prompt Save / Discard / Cancel.
- **Commands**: `workspace.new|switch|close|save|rename|autosave|startup|default|move|open|delete|prompt`; Ctrl+T, Ctrl+W, Ctrl+1..0.

## Capabilities

### New Capabilities
- `workspaces`: tabs, persistence, startup and exit behaviour.


## Impact

- **Code:** `fm-layout` (serde, `relabel`, `validate`, `single_at`), `fm-engine` (`workspace.rs`, `workspace_cmds.rs`), `fm-ui` (`workspaces.rs`, dialogs, preferences), `fm-app` (tab bar, menu, shortcuts, load/save).
- **Dependencies:** none.
- **Risks:** hostile or corrupt files (validated, size-capped, bad entries skipped); pane-id clashes between tabs (disjoint per-tab id ranges).
