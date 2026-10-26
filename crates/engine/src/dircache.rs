//! `DirCache` — the engine's shared directory listings.
//!
//! Keyed by normalized [`VfsUri`]; each slot is `Loading` → (`Ready` | `Failed`)
//! with a strictly monotonic revision. Workers only *stage* results (in the
//! session inbox); slots are rewritten when the UI drains events in
//! [`crate::Session::poll`] — one code path, no stale-entry patches.

use std::collections::HashMap;
use std::sync::Arc;

use fm_vfs::{Entry, VfsUri};

use crate::jobs::JobId;

/// A snapshot of one slot for readers.
#[derive(Clone, Debug, PartialEq)]
pub enum DirState {
    /// No read has completed for this uri.
    Absent,
    /// A read is in flight.
    Loading { job: JobId },
    /// Fresh entries at `revision`.
    Ready {
        revision: u64,
        entries: Arc<[Entry]>,
    },
    /// The last read failed; the message is redacted by `FmError::Display`.
    Failed { revision: u64, message: String },
}

impl DirState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    pub fn entries(&self) -> Option<Arc<[Entry]>> {
        match self {
            Self::Ready { entries, .. } => Some(entries.clone()),
            _ => None,
        }
    }

    pub fn revision(&self) -> Option<u64> {
        match self {
            Self::Ready { revision, .. } | Self::Failed { revision, .. } => Some(*revision),
            Self::Absent | Self::Loading { .. } => None,
        }
    }
}

#[derive(Debug)]
enum Slot {
    Loading {
        job: JobId,
    },
    Ready {
        revision: u64,
        entries: Arc<[Entry]>,
    },
    Failed {
        revision: u64,
        message: String,
    },
}

/// What a staged read produced (written by the worker, read by `poll`).
#[derive(Debug)]
pub enum ReadOutcome {
    Ok(Arc<[Entry]>),
    Err(String),
}

/// Shared, revisioned directory listings.
#[derive(Debug, Default)]
pub struct DirCache {
    slots: HashMap<VfsUri, Slot>,
    next_revision: u64,
}

impl DirCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self, uri: &VfsUri) -> DirState {
        match self.slots.get(uri) {
            None => DirState::Absent,
            Some(Slot::Loading { job }) => DirState::Loading { job: *job },
            Some(Slot::Ready { revision, entries }) => DirState::Ready {
                revision: *revision,
                entries: entries.clone(),
            },
            Some(Slot::Failed { revision, message }) => DirState::Failed {
                revision: *revision,
                message: message.clone(),
            },
        }
    }

    pub fn entries(&self, uri: &VfsUri) -> Option<Arc<[Entry]>> {
        self.state(uri).entries()
    }

    /// Mark `uri` as loading under `job`. Replaces any previous slot.
    pub fn begin_load(&mut self, uri: VfsUri, job: JobId) {
        self.slots.insert(uri, Slot::Loading { job });
    }

    /// Rewrite the slot from a staged outcome. Returns the uri if this job was
    /// the one loading it (so the caller can refresh views), else `None`.
    pub fn apply(&mut self, uri: &VfsUri, outcome: ReadOutcome) -> Option<u64> {
        let is_mine = matches!(self.slots.get(uri), Some(Slot::Loading { .. }));
        if !is_mine {
            return None;
        }
        let revision = self.take_revision();
        let slot = match outcome {
            ReadOutcome::Ok(entries) => Slot::Ready { revision, entries },
            ReadOutcome::Err(message) => Slot::Failed { revision, message },
        };
        self.slots.insert(uri.clone(), slot);
        Some(revision)
    }

    /// Record a read that failed before it reached a job (e.g. no backend for
    /// the scheme), so views show the error affordance instead of a spinner.
    /// Never overwrites an in-flight or fresh slot — those belong to the job.
    /// Swap in fresh entries for a `Ready` slot without passing through `Loading`
    /// (background polls must not blank the listing). `None` when the slot is not
    /// `Ready` or nothing changed.
    pub fn refresh(&mut self, uri: &VfsUri, entries: Arc<[Entry]>) -> Option<u64> {
        match self.slots.get(uri) {
            Some(Slot::Ready { entries: old, .. }) if **old != *entries => {}
            _ => return None,
        }
        let revision = self.take_revision();
        self.slots
            .insert(uri.clone(), Slot::Ready { revision, entries });
        Some(revision)
    }

    pub fn set_failed(&mut self, uri: &VfsUri, message: String) {
        if !matches!(self.slots.get(uri), None | Some(Slot::Failed { .. })) {
            return;
        }
        let revision = self.take_revision();
        self.slots
            .insert(uri.clone(), Slot::Failed { revision, message });
    }

    /// Drop the slot (watcher invalidation / hard refresh).
    pub fn invalidate(&mut self, uri: &VfsUri) {
        self.slots.remove(uri);
    }

    pub fn clear(&mut self) {
        self.slots.clear();
    }

    fn take_revision(&mut self) -> u64 {
        let r = self.next_revision;
        self.next_revision += 1;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::EntryKind;

    fn uri(p: &str) -> VfsUri {
        VfsUri::parse(p).expect("static uri")
    }

    fn entry(name: &str) -> Entry {
        Entry {
            uri: uri(&format!("/d/{name}")),
            name: name.to_string(),
            kind: EntryKind::File,
            size: Some(1),
            modified: None,
            permissions: None,
            hidden: false,
            link_to_dir: false,
        }
    }

    #[test]
    fn lifecycle_absent_loading_ready() {
        let mut c = DirCache::new();
        let u = uri("/d");
        assert_eq!(c.state(&u), DirState::Absent);
        c.begin_load(u.clone(), 7);
        assert_eq!(c.state(&u), DirState::Loading { job: 7 });
        let rev = c
            .apply(&u, ReadOutcome::Ok(Arc::from(vec![entry("a")])))
            .expect("applied");
        assert_eq!(rev, 0);
        let st = c.state(&u);
        assert!(st.is_ready());
        assert_eq!(st.revision(), Some(0));
        assert_eq!(st.entries().map(|e| e.len()), Some(1));
    }

    #[test]
    fn revisions_are_monotonic_across_rereads() {
        let mut c = DirCache::new();
        let u = uri("/d");
        c.begin_load(u.clone(), 1);
        let r1 = c.apply(&u, ReadOutcome::Ok(Arc::from(vec![]))).expect("a");
        c.begin_load(u.clone(), 2);
        let r2 = c
            .apply(&u, ReadOutcome::Ok(Arc::from(vec![entry("x")])))
            .expect("b");
        assert!(r2 > r1);
    }

    #[test]
    fn failed_reads_surface_the_message() {
        let mut c = DirCache::new();
        let u = uri("/locked");
        c.begin_load(u.clone(), 3);
        let rev = c
            .apply(&u, ReadOutcome::Err("permission denied".into()))
            .expect("apply");
        match c.state(&u) {
            DirState::Failed { revision, message } => {
                assert_eq!(revision, rev);
                assert!(message.contains("permission"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn apply_from_a_stale_job_is_ignored() {
        let mut c = DirCache::new();
        let u = uri("/d");
        c.begin_load(u.clone(), 1);
        c.invalidate(&u); // watcher nuked it before the job landed
        assert!(c.apply(&u, ReadOutcome::Ok(Arc::from(vec![]))).is_none());
        assert_eq!(c.state(&u), DirState::Absent);
    }

    #[test]
    fn invalidate_drops_the_slot() {
        let mut c = DirCache::new();
        let u = uri("/d");
        c.begin_load(u.clone(), 1);
        c.apply(&u, ReadOutcome::Ok(Arc::from(vec![entry("a")])));
        c.invalidate(&u);
        assert_eq!(c.state(&u), DirState::Absent);
    }
}
