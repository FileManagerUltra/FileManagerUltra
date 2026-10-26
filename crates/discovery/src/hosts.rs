//! The shared, capped list of discovered hosts.

use std::net::IpAddr;
use std::sync::{Arc, Mutex};

/// Hard cap on remembered hosts (hostile networks can announce unbounded names).
pub const MAX_HOSTS: usize = 512;
const MAX_NAME_CHARS: usize = 64;

/// What a host offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Smb,
    Sftp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Host {
    /// Announcement identity (mDNS full name or `ws:<ip>`), used for removal.
    pub key: String,
    pub name: String,
    pub addr: IpAddr,
    pub smb: Option<u16>,
    pub sftp: Option<u16>,
}

/// Clean an announced name for display: no control characters or path separators, capped.
pub fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\'))
        .take(MAX_NAME_CHARS)
        .collect();
    cleaned.trim().to_string()
}

/// Whether `ip` is usable as a connect target (link-local v6 needs a scope we do not keep).
pub fn usable(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !v4.is_unspecified() && !v4.is_multicast() && !v4.is_loopback(),
        IpAddr::V6(v6) => {
            !v6.is_unspecified()
                && !v6.is_multicast()
                && !v6.is_loopback()
                && (v6.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HostList(Arc<Mutex<Vec<Host>>>);

impl HostList {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `service` for the announcement `key`. Returns `false` if rejected.
    pub fn upsert(&self, key: &str, name: &str, addr: IpAddr, service: Service, port: u16) -> bool {
        let name = sanitize_name(name);
        if name.is_empty() || !usable(&addr) || port == 0 {
            return false;
        }
        let Ok(mut hosts) = self.0.lock() else {
            return false;
        };
        let idx = match hosts.iter().position(|h| h.addr == addr) {
            Some(i) => i,
            None => {
                if hosts.len() >= MAX_HOSTS {
                    return false;
                }
                hosts.push(Host {
                    key: key.to_string(),
                    name: name.clone(),
                    addr,
                    smb: None,
                    sftp: None,
                });
                hosts.len() - 1
            }
        };
        if let Some(h) = hosts.get_mut(idx) {
            // A real name beats the bare address a WS-Discovery reply gives us.
            if h.name == h.addr.to_string() || !key.starts_with("ws:") {
                h.name = name;
                h.key = key.to_string();
            }
            match service {
                Service::Smb => h.smb = Some(port),
                Service::Sftp => h.sftp = Some(port),
            }
        }
        true
    }

    pub fn contains_addr(&self, addr: &IpAddr) -> bool {
        self.0
            .lock()
            .map(|h| h.iter().any(|x| &x.addr == addr))
            .unwrap_or(false)
    }

    /// Drop the host announced under `key` (mDNS goodbye).
    pub fn remove_key(&self, key: &str) {
        if let Ok(mut hosts) = self.0.lock() {
            hosts.retain(|h| h.key != key);
        }
    }

    pub fn snapshot(&self) -> Vec<Host> {
        self.0.lock().map(|h| h.clone()).unwrap_or_default()
    }

    /// Only the caller still holds the list: the folder that owned it is gone.
    pub fn orphaned(&self) -> bool {
        Arc::strong_count(&self.0) == 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("a\u{0}b/c\\d\n"), "abcd");
        assert_eq!(
            sanitize_name(&"x".repeat(500)).chars().count(),
            MAX_NAME_CHARS
        );
        assert_eq!(sanitize_name("  \t "), "");
    }

    #[test]
    fn rejects_unusable_addresses() {
        let l = HostList::new();
        for a in ["0.0.0.0", "127.0.0.1", "224.0.0.1", "fe80::1", "::1"] {
            assert!(!l.upsert("k", "h", ip(a), Service::Smb, 445), "{a}");
        }
        assert!(!l.upsert("k", "h", ip("10.0.0.1"), Service::Smb, 0));
        assert!(!l.upsert("k", "", ip("10.0.0.1"), Service::Smb, 445));
        assert!(l.snapshot().is_empty());
    }

    #[test]
    fn merges_services_and_prefers_real_names() {
        let l = HostList::new();
        l.upsert("ws:10.0.0.2", "10.0.0.2", ip("10.0.0.2"), Service::Smb, 445);
        l.upsert(
            "nas._smb._tcp.local.",
            "nas",
            ip("10.0.0.2"),
            Service::Smb,
            445,
        );
        l.upsert(
            "nas._sftp-ssh._tcp.local.",
            "nas",
            ip("10.0.0.2"),
            Service::Sftp,
            22,
        );
        l.upsert("ws:10.0.0.2", "10.0.0.2", ip("10.0.0.2"), Service::Smb, 445);
        let hosts = l.snapshot();
        assert_eq!(hosts.len(), 1);
        assert_eq!(hosts[0].name, "nas");
        assert_eq!((hosts[0].smb, hosts[0].sftp), (Some(445), Some(22)));
    }

    #[test]
    fn caps_and_removes() {
        let l = HostList::new();
        for i in 0..(MAX_HOSTS + 50) {
            let a = IpAddr::from([10, 1, (i / 250) as u8, (i % 250 + 1) as u8]);
            l.upsert(&format!("k{i}"), "h", a, Service::Smb, 445);
        }
        assert_eq!(l.snapshot().len(), MAX_HOSTS);
        l.remove_key("k0");
        assert_eq!(l.snapshot().len(), MAX_HOSTS - 1);
    }
}
