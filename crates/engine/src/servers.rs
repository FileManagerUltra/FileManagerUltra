//! The `servers:///` folder: saved SFTP profiles as folders, each linking to the server's own
//! `sftp://` root. The list is a snapshot of `Prefs.profiles` kept up to date by the session.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use fm_layout::{PaneId, PaneKind, TreeState};
use fm_vfs::{Entry, EntryKind, FmError, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts};

use crate::prefs::{Profile, Protocol};
use crate::session::Session;

/// One saved server as the folder lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Server {
    pub name: String,
    pub uri: VfsUri,
}

/// Shared between the session (writer) and [`ServersFs`] (reader, on a job lane).
pub type ServerList = Arc<Mutex<Vec<Server>>>;

/// `sftp://user@host[:port]/` for a profile; `None` when the fields do not form an address.
pub fn profile_uri(p: &Profile) -> Option<VfsUri> {
    let port = if p.port == 22 {
        String::new()
    } else {
        format!(":{}", p.port)
    };
    let uri = VfsUri::parse(&format!("sftp://{}@{}{port}/", p.user, p.host)).ok()?;
    let auth = uri.authority();
    (auth.host == p.host && auth.user.as_deref() == Some(p.user.as_str())).then_some(uri)
}

/// The saved SFTP servers, sorted by name; profiles that do not form a valid address are
/// left out.
pub fn servers_of(profiles: &[Profile]) -> Vec<Server> {
    let mut out: Vec<Server> = profiles
        .iter()
        .filter(|p| p.protocol == Protocol::Sftp)
        .filter_map(|p| {
            Some(Server {
                name: p.name.clone(),
                uri: profile_uri(p)?,
            })
        })
        .collect();
    out.sort_by_cached_key(|s| s.name.to_lowercase());
    out
}

impl Session {
    /// Refresh the `servers:///` list from the saved profiles; the folder re-reads when it changed.
    pub(crate) fn sync_servers(&mut self) {
        let fresh = servers_of(&self.prefs.profiles);
        let changed = {
            let Ok(mut list) = self.remote.servers.lock() else {
                return;
            };
            if *list == fresh {
                false
            } else {
                *list = fresh;
                true
            }
        };
        if changed {
            self.dirs.invalidate(&VfsUri::servers());
            if self
                .layout
                .states
                .values()
                .any(|st| st.tree.root.scheme() == Scheme::Servers)
                && let Err(err) = self.refresh_read(VfsUri::servers())
            {
                self.notice = Some(err.to_string());
            }
        }
    }

    /// Point `pane` at the Saved servers folder (tree rooted there).
    pub fn open_servers(&mut self, pane: PaneId) -> Result<(), FmError> {
        self.sync_servers();
        let root = VfsUri::servers();
        let state = self
            .layout
            .state_mut(pane)
            .ok_or_else(|| FmError::NotFound(format!("pane {pane}")))?;
        state.tree = TreeState::new(root.clone());
        state.navigate(root.clone());
        self.refresh_read(root).map(|_| ())
    }

    /// An SFTP pane that is not on a live server (fresh, restored, or just disconnected)
    /// shows the Saved servers folder.
    pub(crate) fn normalize_sftp_panes(&mut self) {
        let stray: Vec<PaneId> = self
            .layout
            .states
            .iter()
            .filter(|(_, st)| st.kind == PaneKind::Sftp && st.tree.root.scheme() != Scheme::Servers)
            .filter(|(_, st)| {
                st.location.scheme() != Scheme::Sftp
                    || (self.backends.get_for(&st.location).is_none()
                        && !matches!(
                            self.remote.state(Scheme::Sftp, st.location.authority()),
                            Some(crate::remote::ConnState::Connecting)
                        )
                        && self.remote.sftp_prompt.is_none())
            })
            .map(|(id, _)| *id)
            .collect();
        for pane in stray {
            if let Err(err) = self.open_servers(pane) {
                self.notice = Some(err.to_string());
            }
        }
    }
}

fn read_only() -> FmError {
    FmError::Unsupported("the Saved servers folder is read-only")
}

#[derive(Debug)]
pub struct ServersFs {
    list: ServerList,
}

impl ServersFs {
    pub fn new(list: ServerList) -> Self {
        Self { list }
    }
}

impl Vfs for ServersFs {
    fn scheme(&self) -> Scheme {
        Scheme::Servers
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        if uri.scheme() != Scheme::Servers {
            return Err(FmError::NotFound(uri.to_string()));
        }
        let list = self
            .list
            .lock()
            .map_err(|_| FmError::Unsupported("servers list"))?;
        Ok(list
            .iter()
            .map(|s| {
                let mut e = Entry::new(s.uri.clone(), EntryKind::Dir);
                e.name = s.name.clone();
                e
            })
            .collect())
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        if uri.scheme() == Scheme::Servers {
            let mut e = Entry::new(uri.clone(), EntryKind::Dir);
            e.name = "Saved servers".into();
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

    fn profile(name: &str, host: &str, port: u16, protocol: Protocol) -> Profile {
        Profile {
            protocol,
            name: name.into(),
            host: host.into(),
            port,
            user: "alice".into(),
            key_path: None,
            start_dir: None,
            domain: None,
            share: None,
        }
    }

    #[test]
    fn lists_only_sftp_profiles_sorted_and_read_only() {
        let profiles = [
            profile("zeta", "z.local", 22, Protocol::Sftp),
            profile("pc", "pc.local", 445, Protocol::Smb),
            profile("Alpha", "a.local", 2222, Protocol::Sftp),
        ];
        let list: ServerList = Arc::new(Mutex::new(servers_of(&profiles)));
        let fs = ServersFs::new(list);
        let root = VfsUri::servers();
        let entries = fs.read_dir(&root).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Alpha", "zeta"]);
        assert_eq!(entries[0].uri.to_string(), "sftp://alice@a.local:2222/");
        assert_eq!(entries[1].uri.to_string(), "sftp://alice@z.local/");
        assert!(fs.read_dir(&VfsUri::network()).is_err());
        assert!(fs.create_dir(&root).is_err());
        assert!(fs.remove(&root).is_err());
        assert!(fs.open_write(&root, WriteOpts::default()).is_err());
    }

    #[test]
    fn profiles_with_unaddressable_fields_are_skipped() {
        let mut bad = profile("bad", "h/ost", 22, Protocol::Sftp);
        bad.user = "a@b".into();
        assert!(servers_of(&[bad]).is_empty());
    }
}
