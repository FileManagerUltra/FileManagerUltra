//! Dragging local files out of the window on Windows: when an in-app drag leaves the window
//! the same files continue as a native OLE drag, so Explorer, browsers, mail and chat
//! clients can receive them. winit can receive drops but cannot start one.
//!
//! The drag blocks inside `DoDragDrop`, so it runs on a helper thread and the UI polls for
//! the result. The files are never read, copied, staged or deleted here; a receiver that
//! performs a "move" does so itself. On other platforms every drag is `Unsupported`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

pub use fm_dragout::{DragError, Outcome};

#[cfg(windows)]
mod ole;
mod paths;

pub use paths::wide_paths;

/// A running drag. Dropping it cancels the drag.
pub struct Drag {
    cancel: Arc<AtomicBool>,
    rx: Receiver<Result<Outcome, DragError>>,
}

impl Drag {
    /// Begin dragging `paths` (absolute local paths) from the current pointer position; the
    /// primary mouse button must be held.
    pub fn start(paths: Vec<PathBuf>) -> Result<Self, DragError> {
        let wide = wide_paths(&paths)?;
        Self::spawn(wide)
    }

    #[cfg(windows)]
    fn spawn(wide: Vec<Vec<u16>>) -> Result<Self, DragError> {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel();
        let flag = cancel.clone();
        std::thread::Builder::new()
            .name("fm-winout".into())
            .spawn(move || {
                let _ = tx.send(ole::run(&wide, flag));
            })
            .map_err(|e| DragError::System(e.to_string()))?;
        Ok(Self { cancel, rx })
    }

    #[cfg(not(windows))]
    fn spawn(_wide: Vec<Vec<u16>>) -> Result<Self, DragError> {
        Err(DragError::Unsupported("only Windows is implemented"))
    }

    /// The outcome once the drag has ended; `None` while it is still running.
    pub fn poll(&self) -> Option<Result<Outcome, DragError>> {
        match self.rx.try_recv() {
            Ok(result) => Some(result),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(DragError::System("the drag thread ended".into())))
            }
        }
    }

    /// Ask the drag to stop; it ends on its next tick.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Drop for Drag {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn other_platforms_report_unsupported_after_validating() {
        assert!(matches!(
            Drag::start(vec![PathBuf::from("/a")]),
            Err(DragError::Unsupported(_))
        ));
        assert!(matches!(
            Drag::start(Vec::new()),
            Err(DragError::BadPayload(_))
        ));
    }
}
