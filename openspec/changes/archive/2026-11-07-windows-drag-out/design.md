## Context

egui/winit can receive OS drops (already used for external drops) but cannot start an OS drag. Linux solved this with `fm-dragout` (XDND) and `fm-wayland`. On Windows the mechanism is OLE: `DoDragDrop(IDataObject, IDropSource, effects)`, which runs a modal loop until the button is released, Escape is pressed or a target accepts.

The workspace forbids `unsafe`. COM interop on Windows cannot be written without it, even with the `windows` crate (its `#[implement]` macro and `DoDragDrop` call are unsafe or expand to unsafe). `fm-wayland` already established the pattern of an isolated, explicitly approved exception.

## Goals / Non-Goals

**Goals:**
- Dropping selected local files from a pane into any Windows application that accepts file drops (Explorer, browsers, Outlook, Teams, Slack).
- No UI-thread blocking, no crash on any failure (`FmError`/`DragError` instead).
- Smallest possible `unsafe` surface, tested for the parts that are pure logic.

**Non-Goals:**
- macOS drag-out.
- Dragging remote (SFTP/SMB) items; these keep the "only local files" notice.
- Virtual files (`CFSTR_FILEDESCRIPTOR`/`FILECONTENTS`) or drag images beyond the OS default cursor.
- Receiving drops (winit already does).

## Decisions

1. **New L0 crate `fm-winout`, Windows-only.** Mirrors `fm-wayland`: own `[lints.rust] unsafe_code = "deny"` with `#![allow(unsafe_code)]` on a single module (`ole.rs`); the rest (`DROPFILES` encoding, path validation) is safe code and unit-tested on every OS. On non-Windows the crate compiles to an empty API returning `Unsupported`, so the workspace builds everywhere. Alternative: put it in `fm-dragout`; rejected because that crate would then need `unsafe` too and lose its guarantee.
2. **`windows` crate with minimal features** (`Win32_System_Ole`, `Win32_System_Com`, `Win32_System_Memory`, `Win32_System_DataExchange`, `Win32_UI_Shell`, `Win32_Foundation`). It is already in `Cargo.lock`; no new vendor. Alternative: raw `extern "system"` declarations; rejected as larger `unsafe` surface.
3. **A minimal `IDataObject` offering only `CF_HDROP`** (global-memory `DROPFILES` block built in safe code, `paths::dropfiles`). A first version used the shell's `SHCreateDataObject`; Chrome accepted the drop but Google's image-search drop zone reported an unknown file type, so the extra formats the shell object advertises are avoided and receivers take the plain file-list route. Paths come from a safe validator (absolute, no NUL, count cap shared with `fm_dragout`, `\\?\\` prefixes stripped). The custom object costs a few more `unsafe` lines, all in `ole.rs`.

4. **Run `DoDragDrop` on the UI thread.** OLE reads the mouse button state and takes capture from the calling thread, which must own the window the press happened in. A first version on a helper thread saw the button as released and ended every drag at once (indicator vanished, nothing dropped). The call blocks the frame loop for the duration of the user's drag, which is inherent to OLE's modal loop; `ReleaseCapture` afterwards returns the mouse to the system because winit never sees the release. API: `fm_winout::drag(paths) -> Result<Outcome>`. Alternative: helper thread; tried and rejected.

5. **Allowed effects: Copy | Move | Link-free.** `DROPEFFECT_COPY | DROPEFFECT_MOVE`; Explorer defaults to Move on the same volume (Shift/Ctrl modify it) and nothing is deleted by us. We do not act on a Move result: files are never removed by the source on a "Move" outcome, to avoid data loss from receivers that misreport. The UI reports "Dropped files into another application".
6. **Start rule unchanged:** `dragout.rs` starts the OS drag when the in-app payload leaves the window with the primary button down; the missed button release is injected like on Linux.

## Risks / Trade-offs

- [`unsafe` in a second crate] → single module, documented invariants, no `unsafe` outside it (`#![deny(unsafe_code)]` elsewhere), reviewed in the PR; listed in `docs/architecture.md` and `AGENTS.md`; removed when winit 0.31 `start_drag` lands.
- [Cannot run in Linux CI] → pure logic is unit-tested everywhere; the OLE path is cross-checked with `cargo check --target x86_64-pc-windows-msvc|gnu` in CI and covered by a manual Windows smoke check in `docs/development.md`.
- [Blocking the UI thread during the drag] → inherent to OLE; bounded by the user's button hold, ended by release or Escape.
- [Receivers that refuse `CF_HDROP` from a temp-less path] → paths are the real files; nothing is copied or staged.
- [Antivirus/SmartScreen prompts] → none expected; no execution or staging.

## Open Questions

- Does OLE's modal loop on the UI thread coexist with winit's message handling (no stuck capture, no event-handler reentrancy)? Resolve with the next Windows build.
