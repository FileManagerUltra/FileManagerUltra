//! Text decoding and the plain-text / source renderer.

use std::io::Read;

use fm_vfs::{Entry, FmError, Vfs};

use crate::{Limits, Preview, TextDoc, highlight};

/// Decode bytes as text: UTF-8 (BOM stripped), UTF-16 by BOM, otherwise lossy UTF-8.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8_lossy(bytes).into_owned()
}

fn utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| f(*c)).collect();
    String::from_utf16_lossy(&units)
}

/// Read at most `cap` bytes; reports whether the file had more.
pub fn read_capped(vfs: &dyn Vfs, entry: &Entry, cap: usize) -> Result<(Vec<u8>, bool), FmError> {
    let mut r = vfs.open_read(&entry.uri)?;
    let mut buf = Vec::new();
    r.by_ref().take(cap as u64 + 1).read_to_end(&mut buf)?;
    let more = buf.len() > cap;
    buf.truncate(cap);
    Ok((buf, more))
}

pub fn make_doc(text: &str, lang: &str, limits: &Limits, truncated: bool, total: u64) -> TextDoc {
    let lines = highlight::highlight(text, lang, limits.text_lines);
    let more_lines = text.lines().nth(limits.text_lines).is_some();
    TextDoc {
        lines,
        lang: lang.to_string(),
        truncated: truncated || more_lines,
        total_bytes: total,
    }
}

pub fn render_text(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    lang: &str,
) -> Result<Preview, FmError> {
    let (bytes, more) = read_capped(vfs, entry, limits.text_bytes)?;
    let total = entry.size.unwrap_or(bytes.len() as u64);
    let text = decode(&bytes);
    Ok(Preview::Text(make_doc(&text, lang, limits, more, total)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_with_crlf() {
        let mut b = vec![0xFF, 0xFE];
        for u in "a\r\nb".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(decode(&b), "a\r\nb");
        let doc = make_doc(&decode(&b), "txt", &Limits::default(), false, 8);
        assert_eq!(doc.lines.len(), 2);
        assert_eq!(doc.lines[0][0].text, "a");
    }

    #[test]
    fn bom_and_lossy() {
        assert_eq!(decode(b"\xEF\xBB\xBFhi"), "hi");
        assert!(decode(&[b'a', 0xFF, b'b']).contains('\u{FFFD}'));
        assert_eq!(decode(&[0xFF, 0xFE, 0x41]), "");
    }
}
