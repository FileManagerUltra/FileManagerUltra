//! Mapping `smb` errors onto [`FmError`]. Transport failures become
//! `Io(ConnectionAborted)` so the engine can tell a dead link from a per-path failure.

use std::io;

use fm_vfs::FmError;

const ACCESS_DENIED: u32 = 0xC000_0022;
const NOT_FOUND: u32 = 0xC000_0034;
const PATH_NOT_FOUND: u32 = 0xC000_003A;
const NAME_COLLISION: u32 = 0xC000_0035;
const SHARING_VIOLATION: u32 = 0xC000_0043;
const DISK_FULL: u32 = 0xC000_007F;
const NOT_A_DIRECTORY: u32 = 0xC000_0103;
const DIR_NOT_EMPTY: u32 = 0xC000_0101;
const NOT_SUPPORTED: u32 = 0xC000_00BB;
const BAD_NETWORK_NAME: u32 = 0xC000_00CC;
const NAME_DELETED: u32 = 0xC000_00C9;
const SESSION_DELETED: u32 = 0xC000_0203;
const LOGON_FAILURE: u32 = 0xC000_006D;
const ACCOUNT_RESTRICTION: u32 = 0xC000_006E;
const PASSWORD_EXPIRED: u32 = 0xC000_0071;
const ACCOUNT_DISABLED: u32 = 0xC000_0072;
const ACCOUNT_LOCKED: u32 = 0xC000_0234;

pub(crate) fn transport_error(msg: &str) -> FmError {
    FmError::Io(io::Error::new(io::ErrorKind::ConnectionAborted, msg))
}

/// `true` for errors that mean the link itself is gone (not a per-path failure).
pub fn is_transport_error(err: &FmError) -> bool {
    matches!(
        err,
        FmError::Io(e) if matches!(
            e.kind(),
            io::ErrorKind::ConnectionAborted
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::TimedOut
                | io::ErrorKind::ConnectionReset
        )
    )
}

pub(crate) fn status_of(e: &smb::Error) -> Option<u32> {
    match e {
        smb::Error::UnexpectedMessageStatus(s) | smb::Error::ReceivedErrorMessage(s, _) => Some(*s),
        _ => None,
    }
}

pub(crate) fn is_auth_status(status: u32) -> bool {
    matches!(
        status,
        LOGON_FAILURE | ACCOUNT_RESTRICTION | PASSWORD_EXPIRED | ACCOUNT_DISABLED | ACCOUNT_LOCKED
    )
}

pub(crate) fn map_err(e: &smb::Error, path: &str) -> FmError {
    if let Some(status) = status_of(e) {
        return match status {
            NOT_FOUND | PATH_NOT_FOUND => FmError::NotFound(path.to_string()),
            ACCESS_DENIED | SHARING_VIOLATION => FmError::PermissionDenied(path.to_string()),
            NAME_COLLISION => FmError::AlreadyExists(path.to_string()),
            DIR_NOT_EMPTY => FmError::NotEmpty(path.to_string()),
            NOT_A_DIRECTORY => FmError::NotADirectory(path.to_string()),
            NOT_SUPPORTED => FmError::Unsupported("the server does not support this operation"),
            DISK_FULL => FmError::backend("smb", format!("{path}: no space left on the share")),
            NAME_DELETED | SESSION_DELETED => transport_error("the session with the server ended"),
            BAD_NETWORK_NAME => FmError::backend("smb", "no such share on this server"),
            s if is_auth_status(s) => FmError::backend("smb", "authentication failed"),
            _ => FmError::backend("smb", format!("{path}: {e}")),
        };
    }
    match e {
        smb::Error::IoError(io) => match io.kind() {
            io::ErrorKind::NotFound => FmError::NotFound(path.to_string()),
            io::ErrorKind::PermissionDenied => FmError::PermissionDenied(path.to_string()),
            io::ErrorKind::AlreadyExists => FmError::AlreadyExists(path.to_string()),
            _ => transport_error("the connection to the server was lost"),
        },
        smb::Error::ConnectionStopped | smb::Error::ChannelRecvError(_) => {
            transport_error("the connection to the server was lost")
        }
        smb::Error::OperationTimeout(..) => {
            transport_error("the server stopped responding (timeout)")
        }
        smb::Error::NotFound(_) => FmError::NotFound(path.to_string()),
        smb::Error::MissingPermissions(_) => FmError::PermissionDenied(path.to_string()),
        smb::Error::SspiError(_) => FmError::backend("smb", "authentication failed"),
        other => FmError::backend("smb", format!("{path}: {other}")),
    }
}
