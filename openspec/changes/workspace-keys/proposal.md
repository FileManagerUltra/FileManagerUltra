## Why

Four keyboard and workspace reports from 2026-11-14: Ctrl+Shift+1 does nothing while Ctrl+Shift+2…9 switch tabs; there is no Ctrl+Tab to move between workspace tabs; Ctrl+Shift+X (Zoom Pane) does nothing on Windows; and copy/cut in one workspace followed by paste in another must work like it does within one workspace.

## What Changes

- **Tab shortcuts match the digit key, not the character it types.** Ctrl+1…9/0 is matched on the physical digit key, so Ctrl+Shift+1…9 behave like Ctrl+1…9 (Shift+1 types `!`, which egui reported as a different key), and layouts that need Shift for digits (AZERTY) work too. Ctrl+Shift+0 stays Zoom Reset.
- **Ctrl+Tab / Ctrl+Shift+Tab switch workspace tabs** (`workspace.next`, `workspace.prev`), everywhere including a focused terminal. The order is a preference, `workspace_tab_order`: `recent` (default; the first press goes back to the previously used tab, further presses while Ctrl stays held go to older ones, releasing Ctrl commits, like Alt+Tab) or `cycle` (tab-bar order, wrapping). Preferences keeps its own Ctrl+Tab for topics while it is open.
- **BREAKING (shortcut):** Ctrl+Shift+Tab no longer swaps panes while a terminal has focus; Tab still swaps panes elsewhere, and the Panes menu works everywhere.
- **Diagnostic log records shortcut chords:** each key press with Ctrl/Cmd/Alt is logged as its key name and modifiers plus the command it ran (or "unmatched"); plain typing is never logged. Used to find why Ctrl+Shift+X does nothing on Windows, then fixed in this change.
- **Copy/cut/paste across workspaces** is covered by tests (copy and cut in one tab, paste in another, cut moves once, the clipboard survives tab switches) and fixed where it is not.

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `workspaces`: shortcut scenario (physical digit keys, Ctrl+Shift+digit) and new requirement "Switching tabs with Ctrl+Tab" (orders, preference, terminal).
- `file-op-commands`: "Cut, copy and paste" gains a cross-workspace scenario.
- `terminal-pane`: the terminal no longer keeps Ctrl+Shift+Tab for pane swapping.
- `diagnostics` (added by change `diagnostics-log`, archive that first): new requirement "Shortcut chords in the diagnostic log".

## Impact

`apps/fm-app/src/menu.rs` (digit matching, Ctrl+Tab handling with Ctrl-release commit, terminal exception, chord logging), `crates/engine` (`workspace.next`/`workspace.prev` commands, recent-use order in `Workspaces`, `workspace_tab_order` pref with display-prefs command), `crates/ui` Preferences row, tests in engine and app. No new dependencies.
