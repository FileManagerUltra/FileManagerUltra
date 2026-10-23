//! Bounded undo stack of inverse records (design D7). Best-effort: a record whose
//! target changed fails with an actionable error and is dropped by the caller.

use std::collections::VecDeque;

use fm_vfs::{FmError, VfsUri};

use crate::ConflictPolicy;
use crate::OpKind;
use crate::ctx::{Backends, OpCtx, OpReport};
use crate::trash::{restore_local, trash_local};

pub const UNDO_CAP: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoRecord {
    /// Items put in the platform trash; undo restores them.
    Trashed { originals: Vec<VfsUri> },
    /// Items moved `from → to`; undo moves them back.
    Moved { pairs: Vec<(VfsUri, VfsUri)> },
    /// One item renamed `from → to`.
    Renamed { from: VfsUri, to: VfsUri },
    /// Items a copy or mkdir brought into existence; undo trashes them.
    Created { uris: Vec<VfsUri> },
}

#[derive(Debug, Default)]
pub struct UndoStack {
    records: VecDeque<UndoRecord>,
}

impl UndoStack {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn push(&mut self, record: UndoRecord) {
        if self.records.len() >= UNDO_CAP {
            self.records.pop_front();
        }
        self.records.push_back(record);
    }

    pub fn pop(&mut self) -> Option<UndoRecord> {
        self.records.pop_back()
    }

    pub fn peek(&self) -> Option<&UndoRecord> {
        self.records.back()
    }

    /// Record the inverse of a finished job. Permanent deletes (no `trashed` items) and
    /// jobs that did nothing push nothing.
    pub fn push_report(&mut self, kind: OpKind, report: &OpReport) {
        match kind {
            OpKind::Copy | OpKind::Mkdir if !report.created.is_empty() => {
                self.push(UndoRecord::Created {
                    uris: report.created.clone(),
                });
            }
            OpKind::Move if !report.moved.is_empty() => {
                self.push(UndoRecord::Moved {
                    pairs: report.moved.clone(),
                });
            }
            OpKind::Rename if !report.moved.is_empty() => {
                for (from, to) in &report.moved {
                    self.push(UndoRecord::Renamed {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
            }
            OpKind::Delete if !report.trashed.is_empty() => {
                self.push(UndoRecord::Trashed {
                    originals: report.trashed.clone(),
                });
            }
            _ => {}
        }
    }
}

fn require_present(backends: &dyn Backends, uri: &VfsUri) -> Result<(), FmError> {
    backends.vfs(uri)?.stat(uri).map(|_| ())
}

fn require_absent(backends: &dyn Backends, uri: &VfsUri) -> Result<(), FmError> {
    match backends.vfs(uri)?.stat(uri) {
        Ok(_) => Err(FmError::AlreadyExists(uri.to_string())),
        Err(FmError::NotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}

fn move_back(backends: &dyn Backends, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
    if backends.vfs(to)?.rename(to, from).is_ok() {
        return Ok(());
    }
    let parent = from
        .parent()
        .ok_or_else(|| FmError::InvalidParams("cannot undo a move into a root".into()))?;
    if to.file_name() != from.file_name() {
        return Err(FmError::Unsupported("undoing a renamed cross-device move"));
    }
    let mut progress = |_: &crate::ctx::Progress| {};
    let mut conflict = |_: &crate::ctx::Conflict| crate::ctx::Decision {
        choice: crate::ctx::Choice::Skip,
        apply_to_all: true,
    };
    let mut ctx = OpCtx::new(ConflictPolicy::Skip, &mut progress, &mut conflict);
    let report = crate::move_(backends, std::slice::from_ref(to), &parent, &mut ctx)?;
    if report.is_clean() && !report.moved.is_empty() {
        Ok(())
    } else {
        Err(FmError::backend("ops", format!("could not move {to} back")))
    }
}

/// Replay one inverse record. Validates every target first so a stale record changes nothing.
pub fn apply(backends: &dyn Backends, record: &UndoRecord) -> Result<(), FmError> {
    match record {
        UndoRecord::Renamed { from, to } => {
            require_present(backends, to)?;
            require_absent(backends, from)?;
            backends.vfs(to)?.rename(to, from)
        }
        UndoRecord::Moved { pairs } => {
            for (from, to) in pairs {
                require_present(backends, to)?;
                require_absent(backends, from)?;
            }
            for (from, to) in pairs {
                move_back(backends, from, to)?;
            }
            Ok(())
        }
        UndoRecord::Created { uris } => {
            for uri in uris {
                require_present(backends, uri)?;
            }
            for uri in uris {
                trash_local(uri)?;
            }
            Ok(())
        }
        UndoRecord::Trashed { originals } => {
            for original in originals {
                restore_local(original)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::SingleBackend;
    use fm_testkit::{FaultFs, MemFs};
    use std::sync::Arc;

    fn uri(s: &str) -> VfsUri {
        VfsUri::parse(s).expect("uri")
    }

    #[test]
    fn stack_is_bounded_and_lifo() {
        let mut stack = UndoStack::new();
        for i in 0..(UNDO_CAP + 10) {
            stack.push(UndoRecord::Created {
                uris: vec![uri(&format!("/f{i}"))],
            });
        }
        assert_eq!(stack.len(), UNDO_CAP);
        let top = stack.pop().expect("top");
        assert_eq!(
            top,
            UndoRecord::Created {
                uris: vec![uri(&format!("/f{}", UNDO_CAP + 9))]
            }
        );
    }

    #[test]
    fn permanent_delete_and_noops_push_nothing() {
        let mut stack = UndoStack::new();
        stack.push_report(OpKind::Delete, &OpReport::default());
        stack.push_report(OpKind::Copy, &OpReport::default());
        assert!(stack.is_empty());
        let trashed = OpReport {
            trashed: vec![uri("/a")],
            ..OpReport::default()
        };
        stack.push_report(OpKind::Delete, &trashed);
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn undo_rename_restores_the_name() {
        let fs = MemFs::new();
        fs.put_str("/d/new.txt", b"x").expect("put");
        let b = SingleBackend(Arc::new(fs.clone()));
        let record = UndoRecord::Renamed {
            from: uri("/d/old.txt"),
            to: uri("/d/new.txt"),
        };
        apply(&b, &record).expect("undo");
        assert!(fs.exists(&uri("/d/old.txt")));
        assert!(!fs.exists(&uri("/d/new.txt")));
    }

    #[test]
    fn undo_move_goes_back_even_without_rename() {
        let fs = FaultFs::new(MemFs::new());
        fs.inner().put_str("/dst/a/f.txt", b"1").expect("put");
        fs.inner().mkdirs_str("/src").expect("src");
        fs.faults().rename_unsupported = true;
        let b = SingleBackend(Arc::new(fs.clone()));
        let record = UndoRecord::Moved {
            pairs: vec![(uri("/src/a"), uri("/dst/a"))],
        };
        apply(&b, &record).expect("undo");
        assert_eq!(fs.inner().get_str("/src/a/f.txt").expect("get"), b"1");
        assert!(!fs.inner().exists(&uri("/dst/a")));
    }

    #[test]
    fn stale_records_error_and_change_nothing() {
        let fs = MemFs::new();
        fs.put_str("/d/a.txt", b"x").expect("put");
        fs.put_str("/d/b.txt", b"y").expect("put");
        let b = SingleBackend(Arc::new(fs.clone()));
        let vanished = UndoRecord::Renamed {
            from: uri("/d/old.txt"),
            to: uri("/d/gone.txt"),
        };
        assert!(matches!(apply(&b, &vanished), Err(FmError::NotFound(_))));
        let occupied = UndoRecord::Renamed {
            from: uri("/d/a.txt"),
            to: uri("/d/b.txt"),
        };
        assert!(matches!(
            apply(&b, &occupied),
            Err(FmError::AlreadyExists(_))
        ));
        let partial = UndoRecord::Created {
            uris: vec![uri("/d/a.txt"), uri("/d/missing.txt")],
        };
        assert!(apply(&b, &partial).is_err());
        assert!(fs.exists(&uri("/d/a.txt")));
        assert_eq!(fs.get_str("/d/b.txt").expect("get"), b"y");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn undo_created_trashes_and_undo_trash_restores() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let Ok(dir) = tempfile::tempdir_in(home) else {
            return;
        };
        let file = dir.path().join("fm-undo-probe.txt");
        std::fs::write(&file, b"probe").expect("write");
        let u = uri(&file.to_string_lossy());
        let b = SingleBackend(Arc::new(fm_fs_local::LocalFs::new()));
        if apply(
            &b,
            &UndoRecord::Created {
                uris: vec![u.clone()],
            },
        )
        .is_err()
        {
            return;
        }
        assert!(!file.exists());
        apply(&b, &UndoRecord::Trashed { originals: vec![u] }).expect("restore");
        assert!(file.exists());
    }
}
