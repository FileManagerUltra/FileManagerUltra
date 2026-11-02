//! SMB backend (change 4b): the pure-Rust `smb` crate (SMB2/3, signing and encryption),
//! blocking, one session and one share per [`SmbFs`]. All SMB work runs on the engine's
//! `smb` lane. Only this crate names `smb` types.

mod connect;
mod errors;
mod fs;
mod shares;

pub use connect::{ConnectParams, DEFAULT_OP_TIMEOUT_SECS, connect, normalize_host};
pub use errors::is_transport_error;
pub use fs::SmbFs;
pub use secrecy::SecretString;
