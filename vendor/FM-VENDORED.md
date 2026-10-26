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
