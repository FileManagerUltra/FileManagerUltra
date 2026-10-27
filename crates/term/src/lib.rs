//! Terminal sessions: a pty-backed shell, a VT parser and a plain cell grid.
//!
//! No UI types and no filesystem access: the engine owns sessions, the UI paints
//! [`Snapshot`]s and sends bytes back. All pty I/O runs on helper threads.

mod charset;
mod emulator;
mod queries;
mod quote;
mod session;
mod shell;

pub use emulator::{Cell, Color, Emulator, MAX_COLS, MAX_ROWS, MIN_SIZE, SCROLLBACK, Snapshot};
pub use quote::{Dialect, cd_command, path_arg};
pub use session::{MAX_INPUT, PUMP_BUDGET, Pump, Status, TermSession, spawn_env_allowed};
pub use shell::Shell;
