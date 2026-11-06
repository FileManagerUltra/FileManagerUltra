//! Dragging local files out of the application into other applications (a browser's drop
//! field, an editor, another file manager).
//!
//! egui/winit can receive OS drops but cannot start one, so this crate is the *source* side
//! of the drag: on X11 (including XWayland) it speaks the XDND protocol on its own
//! connection, in pure Rust. It never touches the filesystem: the receiver gets a
//! `text/uri-list` of the paths it was given.
//!
//! The caller starts a drag while the primary mouse button is held; the drag follows the
//! pointer on a helper thread and ends when the button is released (drop), the target
//! refuses, or [`DragOut::cancel`] is called. Under XWayland the compositor takes the drag
//! over as soon as the selection is owned, so the thread only serves the data. Other platforms report [`DragError::Unsupported`].

use std::fmt;
use std::path::PathBuf;

mod uri;
#[cfg(all(unix, not(target_os = "macos")))]
mod x11;

pub use uri::{MAX_PATHS, uri_list};

/// How a drag ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The receiver accepted the files.
    Dropped,
    /// Released over something that does not take files (or over nothing).
    NoTarget,
    /// Cancelled by the caller.
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragError {
    /// No drag source for this platform/session (e.g. native Wayland, macOS, Windows).
    Unsupported(&'static str),
    /// The payload is empty, too large or has an unusable path.
    BadPayload(String),
    /// The window system refused or failed.
    System(String),
}

impl fmt::Display for DragError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported(why) => write!(f, "dragging files out is not supported here: {why}"),
            Self::BadPayload(why) => write!(f, "cannot drag these files out: {why}"),
            Self::System(why) => write!(f, "drag and drop failed: {why}"),
        }
    }
}

impl std::error::Error for DragError {}

/// A running drag. Dropping it cancels the drag.
pub struct DragOut {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<Result<Outcome, DragError>>>,
}

impl DragOut {
    /// Begin dragging `paths` (absolute local paths) from the current pointer position.
    pub fn start(paths: Vec<PathBuf>) -> Result<Self, DragError> {
        let list = uri_list(&paths)?;
        Self::spawn(list)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn spawn(list: String) -> Result<Self, DragError> {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Connect on the caller's thread so "no X display" is reported synchronously.
        let source = x11::Source::connect(list)?;
        let flag = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("fm-dragout".into())
            .spawn(move || source.run(&flag, &mut x11::real_pointer))
            .map_err(|e| DragError::System(e.to_string()))?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn spawn(_list: String) -> Result<Self, DragError> {
        Err(DragError::Unsupported("only X11 is implemented"))
    }

    /// Whether the drag has ended.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Ask the drag to stop; it ends on its next tick.
    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Wait for the drag to end and report how.
    pub fn finish(mut self) -> Result<Outcome, DragError> {
        match self.thread.take() {
            Some(t) => t
                .join()
                .unwrap_or_else(|_| Err(DragError::System("drag thread panicked".into()))),
            None => Ok(Outcome::Cancelled),
        }
    }
}

impl Drop for DragOut {
    fn drop(&mut self) {
        self.cancel();
    }
}
