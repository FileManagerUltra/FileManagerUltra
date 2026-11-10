use std::io::Read;
use std::process::{Child, Command};
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};

use fm_vfs::FmError;

use super::error;

/// Two queued blocks plus one reader block. Backpressure never expands this bound.
pub(super) struct Pipe {
    child: Child,
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
}

impl Drop for Pipe {
    fn drop(&mut self) {
        // Always on the coordinator worker, never on a UI frame.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
