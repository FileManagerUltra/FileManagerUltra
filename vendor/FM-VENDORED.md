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

## egui_glow 0.37.0 (MIT OR Apache-2.0)

Source: crates.io release 0.37.0 (egui commit 7c0b4040, `crates/egui_glow`), copied
unchanged except `src/painter.rs`.

The local change uploads a frame's geometry once instead of once per mesh. Upstream
calls `glBufferData(STREAM_DRAW)` for every mesh of every frame; with NVIDIA's OpenGL
driver on Windows and "Threaded optimization" on (the default), the driver kept about
25 KB per drawn frame, so the app's commit size grew by gigabytes over a session (found
with the diagnostic log, change `diagnostics-log`; turning the driver option off stopped
it). `paint_primitives` now gathers all meshes into one vertex and one index buffer
(indices rebased), uploads them with `glBufferSubData` into storage that only grows
(`glBufferData` only when a frame needs more room), and draws each mesh as a range.
`EGUI_GLOW_UPLOAD=per-mesh|frame|frame-subdata` selects upstream behaviour, one
`glBufferData` per frame, or the default, for comparisons on a user's machine; the
choice is logged at debug level (`geometry upload: …`).

On each bump, diff `src/painter.rs` against the release, reapply, and check on Windows
with an NVIDIA GPU that commit size stays flat while the window is visible. Offer the
change upstream; drop the patch once egui_glow uploads geometry per frame.
