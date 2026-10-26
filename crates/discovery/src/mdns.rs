//! mDNS / DNS-SD browsing for SMB and SFTP hosts.

use mdns_sd::{ServiceDaemon, ServiceEvent};

use crate::hosts::{HostList, Service};

const SMB: &str = "_smb._tcp.local.";
const SFTP: &str = "_sftp-ssh._tcp.local.";

/// Instance label of `fullname` (`nas._smb._tcp.local.` → `nas`).
pub fn instance_name(fullname: &str, ty_domain: &str) -> String {
    fullname
        .strip_suffix(ty_domain)
        .map(|s| s.trim_end_matches('.'))
        .unwrap_or(fullname)
        .to_string()
}

fn browse(daemon: &ServiceDaemon, ty: &'static str, service: Service, hosts: HostList) {
    let rx = match daemon.browse(ty) {
        Ok(rx) => rx,
        Err(e) => {
            log::debug!("mdns browse {ty}: {e}");
            return;
        }
    };
    let spawned = std::thread::Builder::new()
        .name(format!("fm-mdns{ty}"))
        .spawn(move || {
            while let Ok(event) = rx.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let name = instance_name(&info.fullname, &info.ty_domain);
                        let mut addrs: Vec<_> =
                            info.addresses.iter().map(|a| a.to_ip_addr()).collect();
                        addrs.sort_by_key(|a| a.is_ipv6());
                        for addr in addrs {
                            if hosts.upsert(&info.fullname, &name, addr, service, info.port) {
                                break;
                            }
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => hosts.remove_key(&fullname),
                    _ => {}
                }
            }
        });
    if let Err(e) = spawned {
        log::debug!("mdns thread: {e}");
    }
}

/// Start browsing; the returned daemon must be kept alive for as long as results are wanted.
pub fn start(hosts: &HostList) -> Option<ServiceDaemon> {
    match ServiceDaemon::new() {
        Ok(daemon) => {
            browse(&daemon, SMB, Service::Smb, hosts.clone());
            browse(&daemon, SFTP, Service::Sftp, hosts.clone());
            Some(daemon)
        }
        Err(e) => {
            log::debug!("mdns daemon: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_names() {
        assert_eq!(instance_name("nas._smb._tcp.local.", SMB), "nas");
        assert_eq!(instance_name("my.box._smb._tcp.local.", SMB), "my.box");
        assert_eq!(instance_name("weird", SMB), "weird");
    }
}
