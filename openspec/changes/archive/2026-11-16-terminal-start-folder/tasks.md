## 1. Terminal crate

- [x] 1.1 `TermSession::cwd()` (Linux `/proc/<pid>/cwd`, else start folder / `note_cwd`) and `note_cwd`; test on Linux after `cd`

## 2. Engine

- [x] 2.1 `Session::last_files_dir` updated in `poll`; `pane_dir(pane)`
- [x] 2.2 `ensure_terminal` start-folder order; stored terminal location never used
- [x] 2.3 `term.restart` keeps the folder; `term.cd_here` notes the folder
- [x] 2.4 Tests: split terminal starts in the source terminal's folder (Linux, after `cd`); workspace-independent last Files folder; switching a Files pane to Terminal starts in its folder; restart keeps the folder

## 3. Wrap-up

- [x] 3.1 CHANGELOG, devlog; test/clippy/fmt/layers/brand-check
- [x] 3.2 Manual: split a terminal after `cd`; a terminals-only workspace (passed, user check 2026-11-16)
