# Vendored crates

## sspi 0.21.3

Byte-identical to crates.io except `sspi/Cargo.toml`: the `=` pins on pre-release crypto
crates were relaxed to caret ranges, and the macOS-only `curve25519-dalek`/`ed25519-dalek`
pins were dropped, because they conflict with `russh`'s `curve25519-dalek ^5`.

Rules: keep `Cargo.lock` committed, build CI with `--locked`, and re-diff against the
crates.io release (`diff -r`) on every bump. Only `Cargo.toml` may differ.

## smb-transport 0.12.1

Identical to crates.io except `src/tcp/transport.rs`: `set_nodelay(true)` after connecting.
Without it Nagle + delayed ACK stalled every SMB read request ~40 ms (1.5 MiB/s instead of
300+ MiB/s on loopback). Drop the patch once upstream exposes a nodelay option.

## vt100 0.18.1 (MIT)

Source: <https://github.com/doy/vt100-rust>, crates.io release 0.18.1.
Includes the upstream source, manifests, license, README and changelog.

Local changes are confined to `src/grid.rs`, `src/reflow.rs`, `src/screen.rs`,
`src/lib.rs` and the resize documentation in `src/parser.rs`. Width changes reflow
normal-screen soft wraps and retained history, preserving hard newlines, cell
attributes, wide/combining characters, cursor/saved cursor and the scrolled view's
logical-line anchor. Packing streams rows with a history-capacity-plus-screen
bound; excess oldest history and bottom rows below the cursor's resized screen
are discarded. Height-only normal-screen resizing also preserves pending wrap
and soft-wrap flags when padding trimmed history rows.
Alternate-screen applications keep the upstream non-reflowing resize behavior.

Regression tests live in `crates/term/src/emulator.rs` and run with
`cargo test -p fm-term`, including resize roundtrip properties, arbitrary output,
history eviction and cursor retention. On each bump, diff these files against
the upstream release and rerun the terminal/engine/UI gates. Drop the patch when
upstream provides compatible normal-buffer reflow.

## egui-winit 0.37.0 (MIT OR Apache-2.0)

Identical to crates.io except `src/lib.rs` (change `windows-wgpu`, design D3d): on Windows
`WindowEvent::Moved` no longer asks for a repaint. With Direct3D's flip model a frame
presented during a window move ties each step of the move to that frame, so dragging the
window lagged behind the mouse; the window's contents do not change when it moves, and the
position is read again at the next frame. `EGUI_WINIT_MOVE_REPAINT=1` restores upstream
behaviour for comparison. Other platforms behave as upstream.

Diagnostics (same file): `on_window_event` and `on_mouse_motion` count the events that
asked for a repaint by kind; `take_repaint_events()` hands the counts to the app, which
writes them into the diagnostic log's `repaint=[…]` column as `event:<kind>`.

The version must match eframe's: on every egui bump re-vendor the new release, re-apply
the change (`diff -r` against crates.io shows it) or drop the patch once egui stops
repainting on moves (or offers an option). The upstream macOS scroll-momentum module
keeps its own `unsafe` (AppKit), as in the crates.io release.

