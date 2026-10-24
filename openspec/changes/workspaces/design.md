## Decisions

- **Active layout is `Session::layout`**: existing pane commands are unchanged; inactive tabs park their layout in `Tab.layout`.
- **Disjoint pane-id ranges per tab** keep per-pane engine state (terminals, previews, view indexes) valid for background tabs. Restored layouts are `relabel`led into a fresh range.
- **No I/O in the engine**: `workspace_tick()` returns JSON when the saved set changed; the app writes it (`save_file`) about once a second and on exit, calling `workspace_retry()` on failure.
- **Dirty** = fingerprint (layout without scroll positions) differs from the saved baseline. The untouched built-in tab is ephemeral and never prompts.
- **Known gaps**: tab reorder is session-only; log filters and terminal scrollback are not persisted; a video in a background tab stops and restarts on return.
