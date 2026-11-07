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
3. **Shell data object from the dragged paths** (`SHParseDisplayName` + `SHCreateDataObject`), instead of a hand-written `IDataObject`. The shell object offers the standard file-drop formats (`CF_HDROP`, shell ID list) that Explorer and browsers expect, and keeps our `unsafe` to a handful of calls. Paths come from a safe validator (absolute, no NUL, count cap shared with `fm_dragout`, `\\?\\` prefixes stripped). Alternative: custom `IDataObject` with a `DROPFILES` block; rejected as more `unsafe` and less receiver compatibility.

4. **Run `DoDragDrop` on a helper thread** that calls `OleInitialize` (STA) and owns the data object, like the X11 helper, so the egui frame loop keeps running. Result (`Dropped`/`NoTarget`/`Cancelled`) comes back over a channel polled each frame. Risk: OLE drag loops expect the thread that owns the mouse capture; if the helper-thread approach does not receive the button state, fall back to running on the UI thread (still bounded by the user's button hold). The first smoke check decides; the API (`start` + `poll`) is the same either way.
5. **Allowed effects: Copy | Move | Link-free.** `DROPEFFECT_COPY | DROPEFFECT_MOVE`; Explorer defaults to Move on the same volume (Shift/Ctrl modify it) and nothing is deleted by us. We do not act on a Move result: files are never removed by the source on a "Move" outcome, to avoid data loss from receivers that misreport. The UI reports "Dropped files into another application".
6. **Start rule unchanged:** `dragout.rs` starts the OS drag when the in-app payload leaves the window with the primary button down; the missed button release is injected like on Linux.

## Risks / Trade-offs

- [`unsafe` in a second crate] → single module, documented invariants, no `unsafe` outside it (`#![deny(unsafe_code)]` elsewhere), reviewed in the PR; listed in `docs/architecture.md` and `AGENTS.md`; removed when winit 0.31 `start_drag` lands.
- [Cannot run in Linux CI] → pure logic is unit-tested everywhere; the OLE path is cross-checked with `cargo check --target x86_64-pc-windows-msvc|gnu` in CI and covered by a manual Windows smoke check in `docs/development.md`.
- [Helper-thread OLE may not see mouse capture] → design decision 4 fallback.
- [Receivers that refuse `CF_HDROP` from a temp-less path] → paths are the real files; nothing is copied or staged.
- [Antivirus/SmartScreen prompts] → none expected; no execution or staging.

## Open Questions

- Does `DoDragDrop` work reliably from the helper thread under winit's message pump? Resolve with the first Windows build.
