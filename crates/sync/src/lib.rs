//! Folder synchronisation: scan two folders, build a reviewable plan, run it.
//!
//! Pure library work on top of [`fm_vfs::Vfs`] and `fm-ops`; the engine runs it on its job
//! lanes. Nothing here touches the UI or reads global state.

pub mod compare;
pub mod exec;
pub mod glob;
pub mod plan;
pub mod profile;
pub mod scan;
pub mod snapshot;
pub mod workflow;

pub use exec::{RunOptions, RunReport, SyncProgress, run};
pub use plan::{Action, Guards, Item, Plan, PlanInput, SnapshotState, Status, Summary};
pub use profile::{
    CompareMethod, ConflictRule, DeletePolicy, Filters, OnAttrFailure, Profile, Retention, Side,
    SymlinkPolicy, SyncMode, WatchSettings,
};
pub use scan::{Filter, Node, NodeKind, Scan, ScanLimits, scan};
pub use snapshot::{Loaded, Snapshot, SnapshotStore};
pub use workflow::compare;
