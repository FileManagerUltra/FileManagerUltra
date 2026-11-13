use std::io::Read;
use std::process::{Child, Command};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fm_vfs::FmError;

use super::error;

/// Most bytes of decoder diagnostics kept (the newest ones).
const STDERR_TAIL: usize = 4096;

/// Two queued blocks plus one reader block. Backpressure never expands this bound.
pub(super) struct Pipe {
    child: Child,
    /// The newest diagnostics, when the command pipes stderr.
    stderr: Arc<Mutex<Vec<u8>>>,
    receiver: Receiver<Result<Vec<u8>, FmError>>,
    waiting: Option<Instant>,
    eof: bool,
    ended: bool,
    timeout: Duration,
}

impl Pipe {
    pub(super) fn spawn(
        mut command: Command,
        chunk_bytes: usize,
        timeout: Duration,
    ) -> Result<Self, FmError> {
        let mut child = command.spawn()?;
        let Some(mut out) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("Decoder stdout unavailable"));
        };
        let stderr = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut err) = child.stderr.take() {
            let tail = stderr.clone();
            // Draining keeps ffmpeg from blocking on a full stderr pipe.
            let drained = std::thread::Builder::new()
                .name("media-stderr".into())
                .spawn(move || {
                    let mut chunk = [0_u8; 1024];
                    loop {
                        match err.read(&mut chunk) {
                            Ok(0) => return,
                            Ok(n) => {
                                let Ok(mut tail) = tail.lock() else { return };
                                tail.extend_from_slice(chunk.get(..n).unwrap_or_default());
                                let excess = tail.len().saturating_sub(STDERR_TAIL);
                                tail.drain(..excess);
                            }
                            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                            Err(_) => return,
                        }
                    }
                });
            if let Err(err) = drained {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err.into());
            }
        }
        let (sender, receiver) = sync_channel(2);
        let reader = std::thread::Builder::new()
            .name("media-pipe".into())
            .spawn(move || {
                loop {
                    let mut bytes = vec![0; chunk_bytes];
                    let mut offset = 0;
                    while offset < bytes.len() {
                        match out.read(&mut bytes[offset..]) {
                            Ok(0) => break,
                            Ok(n) => offset += n,
                            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
                            Err(err) => {
                                let _ = sender
                                    .send(Err(error(format!("Cannot read decoder output: {err}"))));
                                return;
                            }
                        }
                    }
                    let tail = offset < chunk_bytes;
                    bytes.truncate(offset);
                    if offset > 0 && sender.send(Ok(bytes)).is_err() {
                        return;
                    }
                    if tail {
                        return;
                    }
                }
            });
        if let Err(err) = reader {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err.into());
        }
        Ok(Self {
            child,
            stderr,
            receiver,
            waiting: None,
            eof: false,
            ended: false,
            timeout,
        })
    }

    pub(super) fn poll(&mut self) -> Result<Option<Vec<u8>>, FmError> {
        if self.ended {
            return Ok(None);
        }
        if !self.eof {
            match self.receiver.try_recv() {
                Ok(bytes) => {
                    self.waiting = None;
                    return bytes.map(Some);
                }
                Err(TryRecvError::Disconnected) => self.eof = true,
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.eof
            && let Some(status) = self.child.try_wait()?
        {
            if !status.success() {
                return Err(error(format!(
                    "Decoder failed ({status}). The file may be corrupt or unsupported."
                )));
            }
            self.ended = true;
            return Ok(None);
        }
        if self.waiting.get_or_insert_with(Instant::now).elapsed() > self.timeout {
            return Err(error(
                "Decoder timed out. Try another file or check ffmpeg.",
            ));
        }
        Ok(None)
    }

    pub(super) fn ended(&self) -> bool {
        self.ended
    }

    /// The newest diagnostics as text (empty unless stderr is piped).
    pub(super) fn diagnostics(&self) -> String {
        self.stderr
            .lock()
            .map(|tail| String::from_utf8_lossy(&tail).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        // Always on the coordinator worker, never on a UI frame.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
