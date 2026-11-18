//! The one-call entry used by the engine: validate, scan both sides, load history, plan.

use std::sync::Arc;

use fm_ops::CancelFlag;
use fm_vfs::{FmError, Vfs};

use crate::plan::{Plan, PlanInput, SnapshotState};
use crate::profile::{Profile, Side, SyncMode};
use crate::scan::{Filter, Scan, ScanLimits, scan};
use crate::snapshot::{Loaded, SnapshotStore};

/// Compare the two folders of `profile`. Never writes. `progress` gets `(side, entries so far)`.
pub fn compare(
    profile: &Profile,
    left: &Arc<dyn Vfs>,
    right: &Arc<dyn Vfs>,
    store: Option<&SnapshotStore>,
    cancel: &CancelFlag,
    progress: &mut dyn FnMut(Side, usize),
) -> Result<Plan, FmError> {
    profile.validate()?;
    let filter = Filter::compile(&profile.filters)?;
    let (lu, ru) = (profile.left_uri()?, profile.right_uri()?);
    let limits = ScanLimits::default();
    let mut side_scan = |vfs: &Arc<dyn Vfs>, uri: &fm_vfs::VfsUri, side: Side| match vfs.stat(uri) {
        // A folder that does not exist yet is an empty side; a run creates it.
        Err(FmError::NotFound(_)) => Ok(Scan {
            missing: true,
            ..Scan::default()
        }),
        _ => scan(
            vfs.as_ref(),
            uri,
            &filter,
            profile.symlinks,
            limits,
            cancel,
            &mut |n| progress(side, n),
        ),
    };
    let l = side_scan(left, &lu, Side::Left)?;
    let r = side_scan(right, &ru, Side::Right)?;
    if l.missing && r.missing {
        return Err(FmError::NotFound("neither folder exists".into()));
    }
    let source_missing = match profile.source {
        Side::Left => l.missing,
        Side::Right => r.missing,
    };
    if profile.mode != SyncMode::TwoWay && source_missing {
        return Err(FmError::NotFound("the source folder does not exist".into()));
    }
    let (snapshot, state) = match (profile.mode, store) {
        (SyncMode::TwoWay, Some(store)) => {
            match store.load(&profile.id, &lu.to_string(), &ru.to_string())? {
                Loaded::Found(s) => (Some(s), SnapshotState::Used),
                Loaded::Missing => (None, SnapshotState::Missing),
                Loaded::Invalid(why) => (None, SnapshotState::Invalid(why)),
            }
        }
        (SyncMode::TwoWay, None) => (None, SnapshotState::Missing),
        _ => (None, SnapshotState::NotUsed),
    };
    Plan::build(PlanInput {
        profile,
        left: &l,
        right: &r,
        left_vfs: left.as_ref(),
        right_vfs: right.as_ref(),
        snapshot: snapshot.as_ref(),
        snapshot_state: state,
        cancel,
    })
}
