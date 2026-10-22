//! The engine: everything the product can *do*.
//!
//! [`Session`] owns application state, the [`CommandSpec`] registry (every user-visible
//! action is a command — `Session::dispatch` is the single entry point), the background
//! job queue (local/smb/sftp lanes) and `prefs` persistence. The UI observes and
//! dispatches; it never performs work itself.

pub mod app_cmds;
pub mod commands;
pub mod jobs;
pub mod prefs;
pub mod registry;
pub mod session;
pub mod theme_cmds;

pub use jobs::{JobCtx, JobEvent, JobId, JobLane, JobQueue};
pub use prefs::{Prefs, ThemePreference};
pub use registry::{CommandSpec, Params, Registry};
pub use session::Session;
