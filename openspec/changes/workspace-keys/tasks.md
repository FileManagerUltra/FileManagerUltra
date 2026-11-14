## 1. Digit shortcuts

- [ ] 1.1 Digit loop matches the physical digit key with Ctrl/Cmd (Shift allowed, Alt not); Ctrl+Shift+0 stays Zoom Reset
- [ ] 1.2 Tests: Ctrl+Shift+1 (key `Exclamationmark`, physical `Num1`) switches to tab 1; Ctrl+Shift+0 resets zoom; Alt+digit does nothing

## 2. Ctrl+Tab

- [ ] 2.1 Engine: `recent` order and `walk` in `Workspaces`; `activate` updates `recent` and ends a walk; closing a tab prunes it
- [ ] 2.2 Commands `workspace.next`, `workspace.prev` (`{hold?: bool}`), `workspace.walk_end`; tests for recent and cycle order, holding, one tab, malformed params
- [ ] 2.3 Pref `workspace_tab_order` (recent|cycle), `prefs.set` key with tests, Preferences › Workspaces row
- [ ] 2.4 App: Ctrl+Tab / Ctrl+Shift+Tab dispatch with `hold: true` everywhere incl. terminal (not in dialogs/text fields); `walk_end` on Ctrl release or focus loss; terminal Ctrl+Shift+Tab pane swap removed; tests

## 3. Ctrl+Shift+X on Windows

- [ ] 3.1 Diagnostic log `key` events for Ctrl/Cmd/Alt chords with the command run or `unmatched`; test that plain typing is not logged
- [ ] 3.2 Windows test: log of Ctrl+Shift+X; fix the cause it shows (manual, needs the user)

## 4. Clipboard across workspaces

- [ ] 4.1 Engine tests: copy in tab 1, paste in tab 2 (twice); cut in tab 1, paste in tab 2 moves once and clears; the clipboard survives new/close/switch; SFTP or SMB source/destination where the test servers exist
- [ ] 4.2 Fix whatever the tests show (cut marking in the listing of another tab, refresh of the source tab)

## 5. Finish

- [ ] 5.1 Docs: workspaces spec text in development.md shortcuts list, CHANGELOG (Added Ctrl+Tab; Fixed Ctrl+Shift+1; Changed terminal Ctrl+Shift+Tab), roadmap to-dos updated
- [ ] 5.2 clippy, tests, `cargo xtask layers`, `cargo xtask brand-check`, fmt, devlog entry
