//! NetBIOS node-status lookup (UDP 137): asks a host for its machine name, so Windows PCs
//! that only answered WS-Discovery show a name instead of a bare address.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::Duration;

const PORT: u16 = 137;
const TIMEOUT: Duration = Duration::from_millis(400);

fn request() -> Vec<u8> {
    let mut q = vec![0x4e, 0x42, 0x00, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
    q.push(32);
    // The wildcard name `*` followed by 15 NULs, half-ASCII encoded.
    q.extend_from_slice(b"CK");
    q.extend(std::iter::repeat_n(b'A', 30));
    q.push(0);
    q.extend_from_slice(&[0x00, 0x21, 0x00, 0x01]);
    q
}

/// The first unique workstation name in a node-status reply. Bounds-checked: the reply is
/// untrusted.
pub fn parse_reply(data: &[u8]) -> Option<String> {
    // Header (12), any echoed questions (34-byte name + type/class), then the answer: a name
    // (2-byte pointer or 34 bytes) + type/class/ttl/rdlength (10) + the name count.
    let questions = usize::from(u16::from_be_bytes([*data.get(4)?, *data.get(5)?]));
    let mut pos = 12 + questions.min(4) * 38;
    pos += if data.get(pos)? & 0xC0 == 0xC0 { 2 } else { 34 };
    pos += 10;
    let count = usize::from(*data.get(pos)?);
    for i in 0..count.min(32) {
        let at = pos + 1 + i * 18;
        let entry = data.get(at..at + 18)?;
        let (name, tail) = entry.split_at(15);
        let suffix = *tail.first()?;
        let flags = u16::from_be_bytes([*tail.get(1)?, *tail.get(2)?]);
        let group = flags & 0x8000 != 0;
        if suffix == 0x00 && !group {
            let text = String::from_utf8_lossy(name);
            let text = text.trim_end_matches([' ', '\0']);
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Ask `ip` for its NetBIOS name; `None` when it does not answer.
pub fn lookup(ip: IpAddr) -> Option<String> {
    let IpAddr::V4(v4) = ip else { return None };
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.set_read_timeout(Some(TIMEOUT)).ok()?;
    socket
        .send_to(&request(), SocketAddr::from((v4, PORT)))
        .ok()?;
    let mut buf = [0u8; 1024];
    let (n, from) = socket.recv_from(&mut buf).ok()?;
    if from.ip() != ip {
        return None;
    }
    parse_reply(buf.get(..n)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_machine_name_and_ignores_groups() {
        let mut r = vec![0u8; 56];
        r.push(2);
        let mut group = b"WORKGROUP      ".to_vec();
        group.extend_from_slice(&[0x00, 0x84, 0x00]);
        let mut host = b"DESKTOP-ABC    ".to_vec();
        host.extend_from_slice(&[0x00, 0x04, 0x00]);
        r.extend(group);
        r.extend(host);
        assert_eq!(parse_reply(&r).as_deref(), Some("DESKTOP-ABC"));
    }

    #[test]
    fn truncated_or_empty_replies_are_none() {
        assert_eq!(parse_reply(&[]), None);
        let mut r = vec![0u8; 56];
        r.push(5);
        r.extend_from_slice(b"short");
        assert_eq!(parse_reply(&r), None);
    }
}
