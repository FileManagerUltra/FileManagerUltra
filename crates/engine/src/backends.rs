//! Scheme → backend registry. The session holds one of these; `submit_read`
//! picks the job lane from the uri's scheme.

use std::collections::HashMap;
use std::sync::Arc;

use fm_fs_local::LocalFs;
use fm_vfs::{FmError, Scheme, Vfs, VfsUri};

use crate::jobs::JobLane;

/// Which lane serves each scheme (architecture §7: lanes, not a shared pool).
pub fn lane_for(scheme: Scheme) -> JobLane {
    match scheme {
        Scheme::File => JobLane::Local,
        Scheme::Smb => JobLane::Smb,
        Scheme::Sftp => JobLane::Sftp,
    }
}

/// Registered backends, one per scheme.
#[derive(Clone, Default)]
pub struct VfsRegistry {
    map: HashMap<Scheme, Arc<dyn Vfs>>,
}

impl std::fmt::Debug for VfsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut schemes: Vec<&str> = self.map.keys().map(|s| s.as_str()).collect();
        schemes.sort_unstable();
        f.debug_struct("VfsRegistry")
            .field("schemes", &schemes)
            .finish()
    }
}

impl VfsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// The production set: local disk always present.
    pub fn with_local() -> Self {
        let mut r = Self::new();
        r.register(Arc::new(LocalFs::new()));
        r
    }

    /// Register (or replace) the backend for its scheme.
    pub fn register(&mut self, backend: Arc<dyn Vfs>) {
        self.map.insert(backend.scheme(), backend);
    }

    pub fn get(&self, scheme: Scheme) -> Option<Arc<dyn Vfs>> {
        self.map.get(&scheme).cloned()
    }

    pub fn require(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        self.get(uri.scheme()).ok_or(FmError::Unsupported(
            "no backend registered for this scheme",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_is_registered_by_default() {
        let r = VfsRegistry::with_local();
        assert!(r.get(Scheme::File).is_some());
        assert!(r.get(Scheme::Sftp).is_none());
        assert!(matches!(
            r.require(&VfsUri::parse("sftp://h/").expect("uri")),
            Err(FmError::Unsupported(_))
        ));
    }

    #[test]
    fn lanes_match_schemes() {
        assert_eq!(lane_for(Scheme::File), JobLane::Local);
        assert_eq!(lane_for(Scheme::Smb), JobLane::Smb);
        assert_eq!(lane_for(Scheme::Sftp), JobLane::Sftp);
    }
}
