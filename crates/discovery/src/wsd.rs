//! WS-Discovery (what Windows hosts announce): multicast probe + `XAddrs` parsing.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::hosts::{HostList, Service, usable};

const WSD_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const PORT: u16 = 3702;
const MAX_DATAGRAM: usize = 16 * 1024;
const MAX_ADDRS: usize = 8;
const MAX_IFACES: usize = 16;
const LISTEN: Duration = Duration::from_secs(4);

fn probe() -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" "#,
            r#"xmlns:wsa="http://schemas.xmlsoap.org/ws/2004/08/addressing" "#,
            r#"xmlns:wsd="http://schemas.xmlsoap.org/ws/2005/04/discovery" "#,
            r#"xmlns:wsdp="http://schemas.xmlsoap.org/ws/2006/02/devprof" "#,
            r#"xmlns:pub="http://schemas.microsoft.com/windows/pub/2005/07">"#,
            r#"<soap:Header><wsa:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</wsa:To>"#,
            r#"<wsa:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</wsa:Action>"#,
            r#"<wsa:MessageID>urn:uuid:{id}</wsa:MessageID></soap:Header>"#,
            r#"<soap:Body><wsd:Probe><wsd:Types>wsdp:Device pub:Computer</wsd:Types></wsd:Probe></soap:Body>"#,
            r#"</soap:Envelope>"#
        ),
        id = message_id()
    )
}

fn message_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{:08x}-{:04x}-4{:03x}-a{:03x}-{:012x}",
        (nanos >> 64) as u32,
        (nanos >> 48) as u16,
        (nanos >> 36) as u16 & 0xfff,
        (nanos >> 24) as u16 & 0xfff,
        nanos as u64 & 0xffff_ffff_ffff
    )
}

/// Extract the host addresses from the `XAddrs` element of a ProbeMatch. Bounded and
/// panic-free: anything that does not parse is skipped.
pub fn parse_xaddrs(datagram: &[u8]) -> Vec<IpAddr> {
    let slice = datagram
        .get(..datagram.len().min(MAX_DATAGRAM))
        .unwrap_or(&[]);
    let text = String::from_utf8_lossy(slice);
    let mut out = Vec::new();
    let mut rest: &str = &text;
    while let Some(pos) = rest.find("XAddrs>") {
        rest = rest.get(pos + "XAddrs>".len()..).unwrap_or("");
        let end = rest.find('<').unwrap_or(rest.len());
        for url in rest.get(..end).unwrap_or("").split_whitespace() {
            if let Some(ip) = host_of(url)
                && usable(&ip)
                && !out.contains(&ip)
                && out.len() < MAX_ADDRS
            {
                out.push(ip);
            }
        }
    }
    out
}

fn host_of(url: &str) -> Option<IpAddr> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    // Scope ids (`fe80::1%eth0`) make the address unusable anyway.
    host.parse().ok()
}

/// A UDP socket bound to `ip` that sends its multicast out of that interface.
fn bind_on(ip: Ipv4Addr) -> std::io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.bind(&SocketAddr::from((ip, 0)).into())?;
    s.set_multicast_if_v4(&ip)?;
    s.set_multicast_ttl_v4(2)?;
    Ok(s.into())
}

/// Probe once, collecting replies for a few seconds.
fn probe_once(hosts: &HostList) -> std::io::Result<()> {
    // One socket per local IPv4 interface: the default route may be a VPN, which would
    // swallow a single unbound multicast probe.
    let mut sockets: Vec<UdpSocket> = Vec::new();
    for iface in if_addrs::get_if_addrs()? {
        if let std::net::IpAddr::V4(ip) = iface.ip()
            && !ip.is_loopback()
            && !ip.is_link_local()
            && sockets.len() < MAX_IFACES
            && let Ok(s) = bind_on(ip)
            && s.set_read_timeout(Some(Duration::from_millis(50))).is_ok()
        {
            sockets.push(s);
        }
    }
    if sockets.is_empty() {
        return Err(std::io::Error::other("no usable network interface"));
    }
    let group = SocketAddr::from((WSD_GROUP, PORT));
    let deadline = Instant::now() + LISTEN;
    let mut next_probe = Instant::now();
    let mut buf = vec![0u8; MAX_DATAGRAM];
    while Instant::now() < deadline {
        // UDP multicast is lossy: repeat the probe while listening.
        if Instant::now() >= next_probe {
            for s in &sockets {
                let _ = s.send_to(probe().as_bytes(), group);
            }
            next_probe += Duration::from_secs(1);
        }
        for s in &sockets {
            let Ok((n, _)) = s.recv_from(&mut buf) else {
                continue;
            };
            let Some(data) = buf.get(..n) else { continue };
            for ip in parse_xaddrs(data) {
                // mDNS knows the real name; only add hosts it has not already named.
                if !hosts.contains_addr(&ip) {
                    hosts.upsert(&format!("ws:{ip}"), &ip.to_string(), ip, Service::Smb, 445);
                    // Windows PCs answer WS-Discovery without a name: ask NetBIOS for it.
                    if let Some(name) = crate::netbios::lookup(ip) {
                        hosts.upsert(&format!("ws:{ip}"), &name, ip, Service::Smb, 445);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Probe every `interval` until the owning folder is dropped.
pub fn run(hosts: HostList, interval: Duration) {
    while !hosts.orphaned() {
        if let Err(e) = probe_once(&hosts) {
            log::debug!("ws-discovery probe failed: {e}");
        }
        let mut waited = Duration::ZERO;
        while waited < interval && !hosts.orphaned() {
            std::thread::sleep(Duration::from_secs(1));
            waited += Duration::from_secs(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_probe_match() {
        let xml = br#"<d:ProbeMatch><d:XAddrs>http://192.168.1.5:5357/abc http://[2001:db8::1]:5357/abc</d:XAddrs></d:ProbeMatch>"#;
        let ips = parse_xaddrs(xml);
        assert_eq!(
            ips,
            vec![
                "192.168.1.5".parse::<IpAddr>().unwrap(),
                "2001:db8::1".parse::<IpAddr>().unwrap()
            ]
        );
    }

    #[test]
    fn rejects_hostile_input() {
        assert!(parse_xaddrs(b"").is_empty());
        assert!(parse_xaddrs(b"XAddrs>").is_empty());
        assert!(
            parse_xaddrs(b"<XAddrs>ftp://1.2.3.4/ http://999.1.1.1/ http://evil.example/</XAddrs>")
                .is_empty()
        );
        assert!(
            parse_xaddrs(b"<XAddrs>http://127.0.0.1/ http://[fe80::1%25eth0]/</XAddrs>").is_empty()
        );
        assert!(parse_xaddrs(&[0xff, 0xfe, b'X', b'A']).is_empty());
        let many: String = (1..60)
            .map(|i| format!("http://10.0.0.{i}/ "))
            .collect::<String>();
        let xml = format!("<XAddrs>{many}</XAddrs>");
        assert_eq!(parse_xaddrs(xml.as_bytes()).len(), MAX_ADDRS);
        let huge = vec![b'<'; 1 << 20];
        assert!(parse_xaddrs(&huge).is_empty());
    }

    #[test]
    fn probe_is_well_formed_text() {
        let p = probe();
        assert!(p.contains("Probe") && p.contains("pub:Computer"));
    }
}
