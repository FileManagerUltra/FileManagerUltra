use fm_vfs::{Entry, FmError, Vfs};

use crate::{HexDoc, Limits, read_head};

pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<HexDoc, FmError> {
    let bytes = read_head(vfs, entry, limits.hex_bytes)?;
    let total = entry.size.unwrap_or(bytes.len() as u64);
    Ok(HexDoc { bytes, total })
}

/// One dump row: offset, hex pairs (padded to 16) and the printable column.
pub fn row(offset: usize, chunk: &[u8]) -> String {
    let mut hex = String::new();
    for i in 0..16 {
        match chunk.get(i) {
            Some(b) => hex.push_str(&format!("{b:02x} ")),
            None => hex.push_str("   "),
        }
        if i == 7 {
            hex.push(' ');
        }
    }
    let ascii: String = chunk
        .iter()
        .map(|b| {
            if (0x20..0x7f).contains(b) {
                *b as char
            } else {
                '.'
            }
        })
        .collect();
    format!("{offset:08x}  {hex} {ascii}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_pads_and_masks() {
        let r = row(16, b"Hi\x00");
        assert!(r.starts_with("00000010  48 69 00 "));
        assert!(r.ends_with("Hi."));
        assert_eq!(row(0, &[0; 16]).matches("00").count(), 20);
    }
}
