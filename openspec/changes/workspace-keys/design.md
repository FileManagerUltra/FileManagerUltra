## Context

Workspace tabs are switched by `workspace.switch {n}` (Ctrl+1..9/0, a digit loop in `apps/fm-app/src/menu.rs` that calls egui's `consume_shortcut`). egui matches shortcuts on the *logical* key and ignores Shift when the pattern has none, so Ctrl+Shift+2 already matched Ctrl+2: Shift+2 types `@`, which egui has no key for, and egui-winit falls back to the physical digit. Shift+1 types `!`, which egui does know (`Key::Exclamationmark`), so Ctrl+Shift+1 never matched. `Workspaces` (`crates/engine/src/workspace.rs`) holds the open tabs and `active`; `Session::activate` swaps layouts. The file clipboard (`Session::clipboard`) is application-wide already.

## Goals / Non-Goals

**Goals:** digit shortcuts that do not depend on what the digit key types; Ctrl+Tab switching with a most-recently-used order (default) or tab order; a way to see on the user's Windows machine where Ctrl+Shift+X goes; cross-workspace clipboard proven by tests.

**Non-Goals:** an Alt+Tab-style overlay listing the tabs while Ctrl is held (the tab bar shows the active tab; can come later); rebinding shortcuts.

## Decisions

**D1: Digits match the physical key.** The digit loop takes `Event::Key` presses whose `physical_key` (or, when absent, `key`) is a digit, with Ctrl/Cmd held and Alt not, Shift allowed, and removes them from the input. It runs after the main table, so Ctrl+Shift+0 is still Zoom Reset (consumed there first). Alternative: also register `Exclamationmark`: layout-specific and misses AZERTY.

**D2: Recent order lives in the engine.** `Workspaces` gets `recent: Vec<u64>` (tab uids, most recent first), updated in `activate` and pruned when a tab closes, and `walk: Option<Walk>` (a snapshot of `recent` plus a position). `workspace.next` / `workspace.prev` take `{hold?: bool}`: in `recent` order they start a walk from the snapshot if none is active, step it, and activate that tab without reordering; `hold: false` (the default, for menus and scripts) ends the walk at once. `workspace.walk_end` ends a walk: the tab reached moves to the front of `recent`. Any other activation (Ctrl+digit, a click, new/close) ends a running walk first. In `cycle` order the commands step `active` with wraparound and ignore `hold`. Alternative: keep the order in the UI: the MRU order must survive frames and be testable without egui, and scripts can switch too.

**D3: Ctrl release commits.** The app dispatches `workspace.next/prev {hold: true}` for Ctrl+Tab / Ctrl+Shift+Tab and, at the start of every frame, `workspace.walk_end` when a walk is active and Ctrl is no longer held or the window lost focus. Ctrl (not Cmd) on every platform, as browsers do on macOS. The keys are handled before the terminal exception, so a terminal no longer sees them, and the terminal-only Ctrl+Shift+Tab pane swap is removed (user decision 2026-11-14). An open dialog (Preferences uses Ctrl+Tab for topics) or text field keeps the keys.

**D4: Preference.** `workspace_tab_order: recent | cycle` in `Prefs` (serde default `recent`), set through `prefs.set` and shown as a choice row in Preferences › Workspaces.

**D5: Chord logging.** When the diagnostic log is on, `consume_shortcuts` notes the frame's key presses with Ctrl/Cmd held (before anything consumes them) and, after dispatching, logs each as `key: <mods>+<key>[ (physical <key>)] -> <ids>|unmatched`. Chords that type characters are skipped (Ctrl+Alt is AltGr on Windows; Alt/Option-only types characters on macOS), and so is everything while a text field has focus: passwords must never reach the log. The Ctrl+Shift+X fix waits for that log line from Windows.

## Risks / Trade-offs

- [Terminal users lose Ctrl+Shift+Tab pane swap] → Tab-free swap remains in the Panes menu; noted as a breaking shortcut change in the changelog.
- [A missed Ctrl release leaves a walk open] → any other activation ends it, and so does focus loss; a stale walk only affects the next Ctrl+Tab.
