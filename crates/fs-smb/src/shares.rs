//! Share enumeration: `NetrShareEnum` (MS-SRVS opnum 15) over the `srvsvc` pipe.
//!
//! The `smb` crate's own `list_shares` binds with NDR64 only, which Samba and most NAS
//! firmwares refuse. This speaks the classic 32-bit NDR transfer syntax that every SMB
//! server supports. The reply is untrusted: every read is bounds-checked and every count
//! capped.

use fm_vfs::FmError;
use smb::resource::Pipe;
use smb::{IoctlBuffer, PipeTransceiveRequest};

use crate::errors::map_err;

/// Longest share name Windows allows is 80; anything longer is not a share.
const MAX_NAME_UNITS: usize = 256;
/// Most shares read from one reply.
const MAX_SHARES: usize = 4096;

const PTYPE_REQUEST: u8 = 0;
const PTYPE_RESPONSE: u8 = 2;
const PTYPE_BIND: u8 = 11;
const PTYPE_BIND_ACK: u8 = 12;
const FIRST_AND_LAST: u8 = 0x03;
const FRAG: u16 = 4280;
const OPNUM_NET_SHARE_ENUM: u16 = 15;

/// `STYPE_DISK`; the high bit marks administrative (`$`) shares.
const STYPE_MASK: u32 = 0x0FFF_FFFF;
const STYPE_DISK: u32 = 0;
const STYPE_SPECIAL: u32 = 0x8000_0000;

const SRVSVC: [u8; 16] = uuid(
    0x4b32_4fc8,
    0x1670,
    0x01d3,
    [0x12, 0x78, 0x5a, 0x47, 0xbf, 0x6e, 0xe1, 0x88],
);
const NDR32: [u8; 16] = uuid(
    0x8a88_5d04,
    0x1ceb,
    0x11c9,
    [0x9f, 0xe8, 0x08, 0x00, 0x2b, 0x10, 0x48, 0x60],
);

const fn uuid(a: u32, b: u16, c: u16, d: [u8; 8]) -> [u8; 16] {
    let a = a.to_le_bytes();
    let b = b.to_le_bytes();
    let c = c.to_le_bytes();
    [
        a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6],
        d[7],
    ]
}

fn header(ptype: u8, len: usize, call_id: u32) -> Vec<u8> {
    let mut h = vec![5, 0, ptype, FIRST_AND_LAST, 0x10, 0, 0, 0];
    h.extend_from_slice(&(len as u16).to_le_bytes());
    h.extend_from_slice(&0u16.to_le_bytes());
    h.extend_from_slice(&call_id.to_le_bytes());
    h
}

fn bind_pdu() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&FRAG.to_le_bytes());
    body.extend_from_slice(&FRAG.to_le_bytes());
    body.extend_from_slice(&0u32.to_le_bytes());
    body.extend_from_slice(&[1, 0, 0, 0]);
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&[1, 0]);
    body.extend_from_slice(&SRVSVC);
    body.extend_from_slice(&3u16.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&NDR32);
    body.extend_from_slice(&2u16.to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    let mut pdu = header(PTYPE_BIND, 16 + body.len(), 1);
    pdu.extend_from_slice(&body);
    pdu
}

fn conformant_string(out: &mut Vec<u8>, text: &str) {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let n = units.len() as u32;
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    for u in units {
        out.extend_from_slice(&u.to_le_bytes());
    }
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn enum_pdu(server: &str) -> Vec<u8> {
    let mut stub = Vec::new();
    stub.extend_from_slice(&0x0002_0000u32.to_le_bytes());
    conformant_string(&mut stub, &format!("\\\\{server}"));
    stub.extend_from_slice(&1u32.to_le_bytes());
    stub.extend_from_slice(&1u32.to_le_bytes());
    stub.extend_from_slice(&0x0002_0004u32.to_le_bytes());
    stub.extend_from_slice(&0u32.to_le_bytes());
    stub.extend_from_slice(&0u32.to_le_bytes());
    stub.extend_from_slice(&u32::MAX.to_le_bytes());
    stub.extend_from_slice(&0x0002_0008u32.to_le_bytes());
    stub.extend_from_slice(&0u32.to_le_bytes());
    let mut body = Vec::new();
    body.extend_from_slice(&(stub.len() as u32).to_le_bytes());
    body.extend_from_slice(&0u16.to_le_bytes());
    body.extend_from_slice(&OPNUM_NET_SHARE_ENUM.to_le_bytes());
    body.extend_from_slice(&stub);
    let mut pdu = header(PTYPE_REQUEST, 16 + body.len(), 2);
    pdu.extend_from_slice(&body);
    pdu
}

/// A bounds-checked little-endian reader.
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let out = self.buf.get(self.at..end)?;
        self.at = end;
        Some(out)
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
    }
    fn align4(&mut self) {
        self.at = self.at.saturating_add(3) & !3;
    }
    fn string(&mut self) -> Option<String> {
        let _max = self.u32()?;
        let offset = self.u32()?;
        let actual = self.u32()? as usize;
        if offset != 0 || actual > MAX_NAME_UNITS {
            return None;
        }
        let raw = self.take(actual.checked_mul(2)?)?;
        self.align4();
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .take_while(|u| *u != 0)
            .collect();
        String::from_utf16(&units).ok()
    }
}

/// Disk shares named in a `NetrShareEnum` response stub (administrative ones left out).
fn parse_shares(stub: &[u8]) -> Option<Vec<String>> {
    let mut r = Reader { buf: stub, at: 0 };
    let level = r.u32()?;
    let switch = r.u32()?;
    if level != 1 || switch != 1 {
        return None;
    }
    if r.u32()? == 0 {
        return Some(Vec::new());
    }
    let count = r.u32()? as usize;
    if count > MAX_SHARES {
        return None;
    }
    if r.u32()? == 0 {
        return Some(Vec::new());
    }
    if r.u32()? as usize != count {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let name_ptr = r.u32()?;
        let kind = r.u32()?;
        let remark_ptr = r.u32()?;
        entries.push((name_ptr != 0, kind, remark_ptr != 0));
    }
    let mut names = Vec::new();
    for (has_name, kind, has_remark) in entries {
        let name = if has_name { r.string() } else { None };
        if has_remark {
            r.string()?;
        }
        let disk = kind & STYPE_MASK == STYPE_DISK && kind & STYPE_SPECIAL == 0;
        if let Some(name) = name
            && disk
        {
            names.push(name);
        }
    }
    Some(names)
}

fn transact(pipe: &Pipe, pdu: Vec<u8>, what: &str) -> Result<Vec<u8>, FmError> {
    let response = pipe
        .fsctl(PipeTransceiveRequest::from(IoctlBuffer::from(pdu)))
        .map_err(|e| map_err(&e, what))?;
    Ok(response.to_vec())
}

fn bad(why: &str) -> FmError {
    FmError::backend("smb", format!("share list: {why}"))
}

/// Ask the server which shares it offers. The `IPC$` share must already be connected.
pub(crate) fn list(pipe: &Pipe, server: &str) -> Result<Vec<String>, FmError> {
    let ack = transact(pipe, bind_pdu(), server)?;
    if ack.get(2) != Some(&PTYPE_BIND_ACK) {
        return Err(bad("server refused the share-list interface"));
    }
    let reply = transact(pipe, enum_pdu(server), server)?;
    if reply.get(2) != Some(&PTYPE_RESPONSE) {
        return Err(bad("server refused the request"));
    }
    if reply
        .get(3)
        .is_some_and(|f| f & FIRST_AND_LAST != FIRST_AND_LAST)
    {
        return Err(bad("reply too large"));
    }
    // 16 byte header, then alloc hint, context id, cancel count and a pad byte.
    let stub = reply.get(24..).ok_or_else(|| bad("short reply"))?;
    parse_shares(stub).ok_or_else(|| bad("malformed reply"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16(out: &mut Vec<u8>, s: &str) {
        conformant_string(out, s);
    }

    fn reply(entries: &[(&str, u32, &str)]) -> Vec<u8> {
        let mut v = Vec::new();
        for x in [
            1u32,
            1,
            0x20000,
            entries.len() as u32,
            0x20004,
            entries.len() as u32,
        ] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        for (i, (_, kind, _)) in entries.iter().enumerate() {
            v.extend_from_slice(&(0x30000 + i as u32 * 8).to_le_bytes());
            v.extend_from_slice(&kind.to_le_bytes());
            v.extend_from_slice(&(0x30004 + i as u32 * 8).to_le_bytes());
        }
        for (name, _, remark) in entries {
            utf16(&mut v, name);
            utf16(&mut v, remark);
        }
        v.extend_from_slice(&[0; 12]);
        v
    }

    #[test]
    fn keeps_plain_disk_shares_only() {
        let stub = reply(&[
            ("media", 0, "Movies"),
            ("C$", 0x8000_0000, "Default share"),
            ("IPC$", 0x8000_0003, "Remote IPC"),
            ("printers", 1, ""),
            ("docs", 0, ""),
        ]);
        assert_eq!(
            parse_shares(&stub),
            Some(vec!["media".into(), "docs".into()])
        );
    }

    #[test]
    fn hostile_replies_are_rejected_not_panicked_on() {
        let good = reply(&[("media", 0, "x")]);
        for cut in 0..good.len() {
            let _ = parse_shares(&good[..cut]);
        }
        let mut huge = good.clone();
        huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_shares(&huge), None);
        assert_eq!(parse_shares(&[0xff; 64]), None);
    }

    #[test]
    fn request_is_well_formed() {
        let pdu = enum_pdu("nas");
        assert_eq!(pdu.get(2), Some(&PTYPE_REQUEST));
        assert_eq!(
            u16::from_le_bytes([pdu[8], pdu[9]]) as usize,
            pdu.len(),
            "fragment length"
        );
        assert_eq!(pdu.len() % 4, 0);
    }
}
