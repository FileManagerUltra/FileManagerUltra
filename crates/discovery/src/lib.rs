//! LAN host discovery and the virtual read-only `network:///` folder.

mod hosts;
mod mdns;
mod netbios;
mod network_fs;
mod wsd;

pub use hosts::{Host, HostList, MAX_HOSTS, Service, sanitize_name};
pub use network_fs::NetworkFs;
pub use wsd::parse_xaddrs;
