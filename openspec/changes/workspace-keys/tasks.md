## 1. Digit shortcuts

- [x] 1.1 Digit loop matches the physical digit key with Ctrl/Cmd (Shift allowed, Alt not); Ctrl+Shift+0 stays Zoom Reset
- [x] 1.2 Tests: Ctrl+Shift+1 (key `Exclamationmark`, physical `Num1`) switches to tab 1; Ctrl+Shift+0 resets zoom; Alt+digit does nothing

## 2. Ctrl+Tab

- [x] 2.1 Engine: `recent` order and `walk` in `Workspaces`; `activate` updates `recent` and ends a walk; closing a tab prunes it
- [x] 2.2 Commands `workspace.next`, `workspace.prev` (`{hold?: bool}`), `workspace.walk_end`; tests for recent and cycle order, holding, one tab, malformed params
- [x] 2.3 Pref `workspace_tab_order` (recent|cycle), `prefs.set` key with tests, Preferences › Workspaces row
- [x] 2.4 App: Ctrl+Tab / Ctrl+Shift+Tab dispatch with `hold: true` everywhere incl. terminal (not in dialogs/text fields); `walk_end` on Ctrl release or focus loss; terminal Ctrl+Shift+Tab pane swap removed; tests

## 3. Ctrl+Shift+X on Windows

- [x] 3.1 Diagnostic log `key` events for Ctrl/Cmd chords with the command run or `unmatched`; never text-typing chords (AltGr, Option) or anything in a text field; test
- [ ] 3.2 Windows test: log of Ctrl+Shift+X; fix the cause it shows (manual, needs the user)

## 4. Clipboard across workspaces

- [x] 4.1 Engine tests: copy in tab 1, paste in tab 2 (twice); cut in tab 1, paste in tab 2 moves once and clears; the clipboard survives new/close/switch (`clipboard_works_across_workspace_tabs`). Remote: the clipboard holds only URIs and no workspace state, and remote paste is covered by `tests/remote.rs`; no extra server test
- [x] 4.2 Fix whatever the tests show: nothing (the listing has no cut marking; the source tab re-reads its folders when activated)

## 5. Finish

- [x] 5.1 Docs: workspaces spec text in development.md shortcuts list, CHANGELOG (Added Ctrl+Tab; Fixed Ctrl+Shift+1; Changed terminal Ctrl+Shift+Tab), roadmap to-dos updated
- [x] 5.2 clippy, tests, `cargo xtask layers`, `cargo xtask brand-check`, fmt, devlog entry
