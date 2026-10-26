//! SSH/SFTP backend (change 4): `russh` + `russh-sftp`, one tokio runtime owned by this
//! crate, bridged to the blocking [`Vfs`](fm_vfs::Vfs) contract with `Handle::block_on`.
//! No other crate depends on tokio.

mod connect;
mod fs;
mod rt;

#[cfg(all(feature = "test-server", unix))]
pub mod testserver;

pub use connect::{
    AuthMethod, ConnectParams, HostKeyInfo, HostKeyVerdict, HostKeyVerifier, RejectAll, connect,
};
pub use fs::SftpFs;
pub use rt::{DEFAULT_KEEPALIVE_SECS, DEFAULT_OP_TIMEOUT_SECS, is_transport_error, set_timeouts};
pub use secrecy::SecretString;
