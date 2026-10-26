//! The crate-wide tokio runtime and the blocking bridge used by every `Vfs` method.

use std::future::Future;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use fm_vfs::FmError;
use tokio::runtime::{Builder, Handle, Runtime};

/// A call that makes no progress for this long fails (hostile / dead server).
pub const DEFAULT_OP_TIMEOUT_SECS: u64 = 10;
/// Seconds between keepalive probes on an idle link (three misses drop it).
pub const DEFAULT_KEEPALIVE_SECS: u64 = 5;

static OP_TIMEOUT_SECS: AtomicU64 = AtomicU64::new(DEFAULT_OP_TIMEOUT_SECS);
static KEEPALIVE_SECS: AtomicU64 = AtomicU64::new(DEFAULT_KEEPALIVE_SECS);

/// Apply the user's timeouts: `op` to every call from now on, `keepalive` to links made
/// from now on. Zero is raised to one second.
pub fn set_timeouts(op_secs: u64, keepalive_secs: u64) {
    OP_TIMEOUT_SECS.store(op_secs.max(1), Ordering::Relaxed);
    KEEPALIVE_SECS.store(keepalive_secs.max(1), Ordering::Relaxed);
}

pub(crate) fn op_timeout() -> Duration {
    Duration::from_secs(OP_TIMEOUT_SECS.load(Ordering::Relaxed))
}

pub(crate) fn keepalive_interval() -> Duration {
    Duration::from_secs(KEEPALIVE_SECS.load(Ordering::Relaxed))
}

static RUNTIME: OnceLock<Result<Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static Runtime, FmError> {
    RUNTIME
        .get_or_init(|| {
            Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("fm-sftp-rt")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| FmError::backend("sftp", format!("cannot start the sftp runtime: {e}")))
}

/// Run `fut` to completion from a job-lane worker. Called from inside a tokio runtime
/// (which would deadlock or panic) it returns an error instead.
pub fn block_on<F: Future>(fut: F) -> Result<F::Output, FmError> {
    if Handle::try_current().is_ok() {
        return Err(FmError::backend(
            "sftp",
            "blocking sftp call attempted from inside the async runtime",
        ));
    }
    Ok(runtime()?.block_on(fut))
}

/// Handle for spawning tasks (the in-process test server) on the shared runtime.
#[cfg(all(feature = "test-server", unix))]
pub fn handle() -> Result<Handle, FmError> {
    Ok(runtime()?.handle().clone())
}

/// [`block_on`] with the idle timeout applied.
pub fn block_on_timeout<F: Future>(fut: F) -> Result<F::Output, FmError> {
    match block_on(async { tokio::time::timeout(op_timeout(), fut).await })? {
        Ok(v) => Ok(v),
        Err(_) => Err(transport_error("the server stopped responding (timeout)")),
    }
}

/// A broken or stalled link: the engine treats this as "connection lost".
pub fn transport_error(msg: &str) -> FmError {
    FmError::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionAborted,
        msg.to_string(),
    ))
}

/// `true` for errors that mean the link itself is gone (not a per-path failure).
pub fn is_transport_error(err: &FmError) -> bool {
    use std::io::ErrorKind as K;
    matches!(
        err,
        FmError::Io(e) if matches!(
            e.kind(),
            K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof | K::TimedOut | K::ConnectionReset
        )
    )
}
