//! Scheme → backend registry. The session holds one of these; `submit_read`
//! picks the job lane from the uri's scheme.

use std::collections::HashMap;
use std::sync::Arc;

use fm_fs_local::LocalFs;
use fm_vfs::{Authority, FmError, Scheme, Vfs, VfsUri};

use crate::jobs::JobLane;

/// Which lane serves each scheme (architecture §7: lanes, not a shared pool).
pub fn lane_for(scheme: Scheme) -> JobLane {
    match scheme {
        Scheme::File | Scheme::Network | Scheme::Servers => JobLane::Local,
        Scheme::Smb => JobLane::Smb,
        Scheme::Sftp => JobLane::Sftp,
    }
}

/// The lane for long file operations on `scheme`, apart from the lane that serves listings.
pub fn bulk_lane_for(scheme: Scheme) -> JobLane {
    match lane_for(scheme) {
        JobLane::Smb => JobLane::BulkSmb,
        JobLane::Sftp => JobLane::BulkSftp,
        _ => JobLane::BulkLocal,
    }
}

/// Registered backends, keyed by `(scheme, authority)`; the authority is `""` for
/// scheme-wide backends (local disk, in-memory test filesystems).
#[derive(Clone, Default)]
pub struct VfsRegistry {
    map: HashMap<(Scheme, String), Arc<dyn Vfs>>,
    /// The Network folder, kept typed so hosts reached by address can join it.
    network: Option<Arc<fm_discovery::NetworkFs>>,
}

impl std::fmt::Debug for VfsRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut schemes: Vec<String> = self
            .map
            .keys()
            .map(|(s, a)| format!("{}://{a}", s.as_str()))
            .collect();
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
        r.register_network(Arc::new(fm_discovery::NetworkFs::new()));
        r
    }

    /// Register the Network folder (also reachable as the `network` scheme backend).
    pub fn register_network(&mut self, network: Arc<fm_discovery::NetworkFs>) {
        self.register(network.clone());
        self.network = Some(network);
    }

    /// The Network folder, when one is registered.
    pub fn network(&self) -> Option<&Arc<fm_discovery::NetworkFs>> {
        self.network.as_ref()
    }

    /// Register (or replace) the backend for its scheme.
    pub fn register(&mut self, backend: Arc<dyn Vfs>) {
        if backend.scheme() == Scheme::Network {
            self.network = None;
        }
        self.map.insert((backend.scheme(), String::new()), backend);
    }

    /// Register (or replace) the backend serving one `authority` (`user@host:port`).
    pub fn register_at(&mut self, authority: &Authority, backend: Arc<dyn Vfs>) {
        self.map
            .insert((backend.scheme(), authority.to_string()), backend);
    }

    /// Drop the backend for one authority; returns whether one was registered.
    pub fn unregister_at(&mut self, scheme: Scheme, authority: &Authority) -> bool {
        self.map.remove(&(scheme, authority.to_string())).is_some()
    }

    /// The scheme-wide backend.
    pub fn get(&self, scheme: Scheme) -> Option<Arc<dyn Vfs>> {
        self.map.get(&(scheme, String::new())).cloned()
    }

    /// Backend for a uri: the exact authority first, then the scheme-wide one.
    pub fn get_for(&self, uri: &VfsUri) -> Option<Arc<dyn Vfs>> {
        self.map
            .get(&(uri.scheme(), uri.authority().to_string()))
            .or_else(|| self.map.get(&(uri.scheme(), String::new())))
            .cloned()
    }

    pub fn require(&self, uri: &VfsUri) -> Result<Arc<dyn Vfs>, FmError> {
        self.get_for(uri).ok_or(FmError::Unsupported(
            "no backend registered for this location",
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
    fn authority_keyed_backends_are_isolated() {
        let mut r = VfsRegistry::with_local();
        let local = VfsUri::parse("file:///x").expect("uri");
        let h1 = VfsUri::parse("sftp://u@h1/x").expect("uri");
        let h2 = VfsUri::parse("sftp://u@h2/x").expect("uri");
        // A MemFs reports the `file` scheme, so key it as an sftp backend by hand.
        let mem: Arc<dyn Vfs> = Arc::new(fm_testkit::MemFs::new());
        r.map
            .insert((Scheme::Sftp, h1.authority().to_string()), mem);
        assert!(r.require(&local).is_ok());
        assert!(r.require(&h1).is_ok());
        assert!(r.require(&h2).is_err());
        assert!(r.get(Scheme::Sftp).is_none());
        assert!(r.unregister_at(Scheme::Sftp, h1.authority()));
        assert!(!r.unregister_at(Scheme::Sftp, h1.authority()));
        assert!(r.require(&h1).is_err());
    }

    #[test]
    fn lanes_match_schemes() {
        assert_eq!(lane_for(Scheme::File), JobLane::Local);
        assert_eq!(lane_for(Scheme::Smb), JobLane::Smb);
        assert_eq!(lane_for(Scheme::Sftp), JobLane::Sftp);
    }
}
