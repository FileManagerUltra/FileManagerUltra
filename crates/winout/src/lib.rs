//! Dragging local files out of the window on Windows: when an in-app drag leaves the window
//! the same files continue as a native OLE drag, so Explorer, browsers, mail and chat
//! clients can receive them. winit can receive drops but cannot start one.
//!
//! [`drag`] blocks inside `DoDragDrop` until the button is released or Escape is pressed,
//! so it must run on the UI thread that owns the window the button was pressed in (OLE reads
//! the button state and takes the mouse capture from the calling thread). The files are
//! never read, copied, staged or deleted here; a receiver that performs a "move" does so
//! itself. On other platforms every drag is `Unsupported`.

use std::path::PathBuf;

pub use fm_dragout::{DragError, Outcome};

#[cfg(windows)]
mod ole;
mod paths;

pub use paths::wide_paths;

/// Drag `paths` (absolute local paths) from the current pointer position and wait for the
/// result. The primary mouse button must be held.
pub fn drag(paths: Vec<PathBuf>) -> Result<Outcome, DragError> {
    let wide = wide_paths(&paths)?;
    run(&wide)
}

#[cfg(windows)]
fn run(wide: &[Vec<u16>]) -> Result<Outcome, DragError> {
    ole::run(wide)
}

#[cfg(not(windows))]
fn run(_wide: &[Vec<u16>]) -> Result<Outcome, DragError> {
    Err(DragError::Unsupported("only Windows is implemented"))
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn other_platforms_report_unsupported_after_validating() {
        assert!(matches!(
            drag(vec![PathBuf::from("/a")]),
            Err(DragError::Unsupported(_))
        ));
        assert!(matches!(drag(Vec::new()), Err(DragError::BadPayload(_))));
    }
}
