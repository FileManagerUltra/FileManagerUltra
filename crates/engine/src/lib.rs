//! The engine: everything the product can *do*.
//!
//! [`Session`] owns application state, the [`CommandSpec`] registry (every user-visible
//! action is a command — `Session::dispatch` is the single entry point), the background
//! job queue (local/smb/sftp lanes) and `prefs` persistence. The UI observes and
//! dispatches; it never performs work itself.

pub mod app_cmds;
pub mod arrange_cmds;
pub mod backends;
pub mod commands;
pub mod dircache;
pub mod dnd_cmds;
pub mod file_cmds;
pub mod fileops;
pub mod goto_cmds;
pub mod job_cmds;
pub mod jobs;
pub mod nav_cmds;
pub mod pane_cmds;
pub mod pane_kind_cmds;
pub mod prefs;
pub mod prefs_cmds;
pub mod preview_cmds;
pub mod registry;
pub mod session;
pub mod sort_cmds;
pub mod term_cmds;
pub mod theme_cmds;
pub mod view_cmds;

pub use backends::{VfsRegistry, lane_for};
pub use dircache::{DirCache, DirState, ReadOutcome};
pub use dnd_cmds::{DropAction, DropModifiers, DropOrigin, DropResolution, resolve_drop};
pub use fileops::{FileOps, OpJob, OpRequest, OpState, OpSummary};
pub use jobs::{JobCtx, JobEvent, JobId, JobLane, JobQueue};
pub use prefs::{Prefs, ThemePreference};
pub use registry::{CommandSpec, Params, Registry};
pub use session::{Session, ViewIndex, is_hidden};
