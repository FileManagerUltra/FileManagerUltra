use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use fm_vfs::{Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};
use mdns_sd::ServiceDaemon;

use crate::hosts::{Host, HostList, sanitize_name};

const WSD_INTERVAL: Duration = Duration::from_secs(10);
/// Hosts reached by address (typed `\\host` or `smb://host`) kept in the folder this run.
pub const MAX_VISITED: usize = 64;

fn read_only() -> FmError {
    FmError::Unsupported("the Network folder is read-only")
}

/// The `network:///` folder: one entry per discovered host and service, each linking to
/// the host's own `smb://` / `sftp://` root. Discovery starts on the first listing.
#[derive(Default)]
pub struct NetworkFs {
    hosts: HostList,
    started: OnceLock<()>,
    daemon: Mutex<Option<ServiceDaemon>>,
    /// Host roots opened by address that discovery may never announce, oldest first.
    visited: Mutex<Vec<VfsUri>>,
}

impl std::fmt::Debug for NetworkFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkFs").finish_non_exhaustive()
    }
}

impl NetworkFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// A folder over a fixed host list that never touches the network (tests).
    pub fn fixed(hosts: HostList) -> Self {
        let fs = Self {
            hosts,
            ..Self::default()
        };
        let _ = fs.started.set(());
        fs
    }

    /// The live host list (tests and the engine can seed or inspect it).
    pub fn hosts(&self) -> &HostList {
        &self.hosts
    }

    /// Keep the host of `uri` (an `smb://` or `sftp://` location that just listed) in the
    /// folder, so a host typed by address shows in the Network tree. `true` when it is new.
    /// Capped at [`MAX_VISITED`]; the oldest host makes room.
    pub fn remember_host(&self, uri: &VfsUri) -> bool {
        if !matches!(uri.scheme(), Scheme::Smb | Scheme::Sftp) {
            return false;
        }
        let mut root = uri.clone();
        while let Some(parent) = root.parent() {
            root = parent;
        }
        let Ok(mut visited) = self.visited.lock() else {
            return false;
        };
        if visited.contains(&root) {
            return false;
        }
        if visited.len() >= MAX_VISITED {
            visited.remove(0);
        }
        visited.push(root);
        true
    }

    fn start(&self) {
        if self.started.set(()).is_err() {
            return;
        }
        if let Ok(mut slot) = self.daemon.lock() {
            *slot = crate::mdns::start(&self.hosts);
        }
        let hosts = self.hosts.clone();
        if let Err(e) = std::thread::Builder::new()
            .name("fm-wsd".into())
            .spawn(move || crate::wsd::run(hosts, WSD_INTERVAL))
        {
            log::debug!("ws-discovery thread: {e}");
        }
    }
}

fn host_entries(host: &Host) -> Vec<Entry> {
    let mut out = Vec::new();
    let addr = match host.addr {
        std::net::IpAddr::V4(a) => a.to_string(),
        std::net::IpAddr::V6(a) => format!("[{a}]"),
    };
    let mut push = |scheme: &str, port: u16, default: u16, label: &str| {
        let port = if port == default {
            String::new()
        } else {
            format!(":{port}")
        };
        if let Ok(uri) = VfsUri::parse(&format!("{scheme}://{addr}{port}/")) {
            let mut e = Entry::new(uri, EntryKind::Dir);
            e.name = format!("{} ({label})", host.name);
            out.push(e);
        }
    };
    if let Some(p) = host.smb {
        push("smb", p, 445, "File Sharing");
    }
    if let Some(p) = host.sftp {
        push("sftp", p, 22, "Remote Login");
    }
    out
}

impl Vfs for NetworkFs {
    fn scheme(&self) -> Scheme {
        Scheme::Network
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        if uri.scheme() != Scheme::Network {
            return Err(FmError::NotFound(uri.to_string()));
        }
        self.start();
        let mut entries: Vec<Entry> = self
            .hosts
            .snapshot()
            .iter()
            .flat_map(host_entries)
            .collect();
        let visited = self.visited.lock().map(|v| v.clone()).unwrap_or_default();
        for root in visited {
            if entries.iter().any(|e| e.uri == root) {
                continue;
            }
            let label = if root.scheme() == Scheme::Smb {
                "File Sharing"
            } else {
                "Remote Login"
            };
            let mut e = Entry::new(root.clone(), EntryKind::Dir);
            e.name = format!("{} ({label})", sanitize_name(&root.authority().host));
            entries.push(e);
        }
        entries.sort_by_cached_key(|e| (e.name.to_lowercase(), e.uri.to_string()));
        Ok(entries)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        if uri.scheme() == Scheme::Network {
            let mut e = Entry::new(uri.clone(), EntryKind::Dir);
            e.name = "Network".into();
            Ok(e)
        } else {
            Err(FmError::NotFound(uri.to_string()))
        }
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        Err(FmError::NotADirectory(uri.to_string()))
    }

    fn open_write(&self, _: &VfsUri, _: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        Err(read_only())
    }

    fn create_dir(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(read_only())
    }

    fn remove(&self, _: &VfsUri) -> Result<(), FmError> {
        Err(read_only())
    }

    fn rename(&self, _: &VfsUri, _: &VfsUri) -> Result<(), FmError> {
        Err(read_only())
    }

    fn same_device(&self, _: &VfsUri, _: &VfsUri) -> bool {
        false
    }

    fn space(&self, _: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hosts::Service;

    #[test]
    fn lists_hosts_as_links_and_is_read_only() {
        let fs = NetworkFs::new();
        // Mark started so tests never touch the network.
        let _ = fs.started.set(());
        let ip = "192.168.1.9".parse().unwrap();
        fs.hosts().upsert("k", "nas", ip, Service::Smb, 445);
        fs.hosts().upsert("k", "nas", ip, Service::Sftp, 2222);
        let net = VfsUri::network();
        let list = fs.read_dir(&net).unwrap();
        let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["nas (File Sharing)", "nas (Remote Login)"]);
        assert_eq!(list[0].uri.to_string(), "smb://192.168.1.9/");
        assert_eq!(list[1].uri.to_string(), "sftp://192.168.1.9:2222/");
        let file = VfsUri::parse("file:///tmp").unwrap();
        assert!(fs.read_dir(&file).is_err());
        assert!(fs.create_dir(&net).is_err());
        assert!(fs.remove(&net).is_err());
        assert!(fs.open_write(&net, WriteOpts::default()).is_err());
        assert!(fs.open_read(&net).is_err());
    }

    #[test]
    fn hosts_reached_by_address_join_the_folder_once_and_are_capped() {
        let fs = NetworkFs::fixed(HostList::new());
        let ip = "192.168.0.9".parse().unwrap();
        fs.hosts().upsert("k", "nas", ip, Service::Smb, 445);
        let net = VfsUri::network();
        let share = VfsUri::parse("smb://192.168.0.5/media/films").unwrap();
        assert!(fs.remember_host(&share));
        assert!(!fs.remember_host(&share), "already known");
        // A discovered host is not listed twice.
        assert!(fs.remember_host(&VfsUri::parse("smb://192.168.0.9/x").unwrap()));
        // Only remote hosts count.
        assert!(!fs.remember_host(&VfsUri::parse("file:///tmp").unwrap()));
        assert!(!fs.remember_host(&net));
        let list = fs.read_dir(&net).unwrap();
        let shown: Vec<_> = list
            .iter()
            .map(|e| (e.name.as_str(), e.uri.to_string()))
            .collect();
        assert_eq!(
            shown,
            [
                (
                    "192.168.0.5 (File Sharing)",
                    "smb://192.168.0.5/".to_string()
                ),
                ("nas (File Sharing)", "smb://192.168.0.9/".to_string()),
            ]
        );
        for n in 0..MAX_VISITED + 10 {
            let uri = VfsUri::parse(&format!("sftp://10.0.{}.{}/", n / 200, n % 200 + 1)).unwrap();
            fs.remember_host(&uri);
        }
        assert!(fs.read_dir(&net).unwrap().len() <= MAX_VISITED + 1);
    }
}
