//! PDF text strings (ISO 32000-1 7.9.2): UTF-16BE or UTF-8 with a byte-order mark,
//! otherwise PDFDocEncoding.

/// Longest text string decoded for titles, labels and field values.
pub const MAX_TEXT: usize = 32 * 1024;

/// Decode a PDF text string, capped at [`MAX_TEXT`] bytes of input.
pub fn decode_text(bytes: &[u8]) -> String {
    let bytes = bytes.get(..bytes.len().min(MAX_TEXT)).unwrap_or(&[]);
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        return strip_controls(&String::from_utf16_lossy(&units));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        // Not allowed by the spec, but written by some producers.
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return strip_controls(&String::from_utf16_lossy(&units));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return strip_controls(&String::from_utf8_lossy(rest));
    }
    strip_controls(&bytes.iter().map(|b| pdfdoc_char(*b)).collect::<String>())
}

/// Encode text as a PDF text string: PDFDocEncoding when every character fits, else
/// UTF-16BE with a byte-order mark.
pub fn encode_text(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        match pdfdoc_byte(ch) {
            Some(b) => out.push(b),
            None => {
                let mut utf16 = vec![0xFE, 0xFF];
                for unit in text.encode_utf16() {
                    utf16.extend_from_slice(&unit.to_be_bytes());
                }
                return utf16;
            }
        }
    }
    out
}

/// Language escape sequences (U+001B … U+001B) and other controls except tab/newline.
fn strip_controls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for ch in s.chars() {
        if ch == '\u{1b}' {
            in_escape = !in_escape;
            continue;
        }
        if in_escape {
            continue;
        }
        if ch.is_control() && ch != '\n' && ch != '\t' {
            if ch == '\r' {
                out.push('\n');
            }
            continue;
        }
        out.push(ch);
    }
    out
}

/// PDFDocEncoding byte → char (Annex D.2); undefined codes map to U+FFFD.
pub fn pdfdoc_char(b: u8) -> char {
    match b {
        0x18 => '\u{02D8}',
        0x19 => '\u{02C7}',
        0x1A => '\u{02C6}',
        0x1B => '\u{02D9}',
        0x1C => '\u{02DD}',
        0x1D => '\u{02DB}',
        0x1E => '\u{02DA}',
        0x1F => '\u{02DC}',
        0x80 => '\u{2022}',
        0x81 => '\u{2020}',
        0x82 => '\u{2021}',
        0x83 => '\u{2026}',
        0x84 => '\u{2014}',
        0x85 => '\u{2013}',
        0x86 => '\u{0192}',
        0x87 => '\u{2044}',
        0x88 => '\u{2039}',
        0x89 => '\u{203A}',
        0x8A => '\u{2212}',
        0x8B => '\u{2030}',
        0x8C => '\u{201E}',
        0x8D => '\u{201C}',
        0x8E => '\u{201D}',
        0x8F => '\u{2018}',
        0x90 => '\u{2019}',
        0x91 => '\u{201A}',
        0x92 => '\u{2122}',
        0x93 => '\u{FB01}',
        0x94 => '\u{FB02}',
        0x95 => '\u{0141}',
        0x96 => '\u{0152}',
        0x97 => '\u{0160}',
        0x98 => '\u{0178}',
        0x99 => '\u{017D}',
        0x9A => '\u{0131}',
        0x9B => '\u{0142}',
        0x9C => '\u{0153}',
        0x9D => '\u{0161}',
        0x9E => '\u{017E}',
        0x9F => '\u{FFFD}',
        0xA0 => '\u{20AC}',
        0x7F | 0xAD => '\u{FFFD}',
        b => char::from(b),
    }
}

/// char → PDFDocEncoding byte, when it has one.
pub fn pdfdoc_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    if (0x20..0x7F).contains(&code) || ch == '\n' || ch == '\t' || ch == '\r' {
        return Some(code as u8);
    }
    if (0xA1..=0xFF).contains(&code) && code != 0xAD {
        return Some(code as u8);
    }
    (0x18u8..=0xA0)
        .find(|b| !b.is_ascii() || *b < 0x20)
        .filter(|b| pdfdoc_char(*b) == ch)
}

/// WinAnsiEncoding (Annex D.2) char → byte, for simple fonts in form fields.
pub fn winansi_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    if (0x20..0x7F).contains(&code) {
        return Some(code as u8);
    }
    if (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    let b = match ch {
        '\u{20AC}' => 0x80,
        '\u{201A}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201E}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02C6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8A,
        '\u{2039}' => 0x8B,
        '\u{0152}' => 0x8C,
        '\u{017D}' => 0x8E,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201C}' => 0x93,
        '\u{201D}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02DC}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9A,
        '\u{203A}' => 0x9B,
        '\u{0153}' => 0x9C,
        '\u{017E}' => 0x9E,
        '\u{0178}' => 0x9F,
        _ => return None,
    };
    Some(b)
}

/// Page label numbering (ISO 32000-1 12.4.2).
pub fn roman(mut n: u32, upper: bool) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if n == 0 || n > 100_000 {
        return n.to_string();
    }
    let mut out = String::new();
    for (value, text) in TABLE {
        while n >= value {
            out.push_str(text);
            n -= value;
        }
    }
    if upper { out.to_uppercase() } else { out }
}

/// `a`..`z`, `aa`..`zz`, … as page labels number letters.
pub fn letters(n: u32, upper: bool) -> String {
    if n == 0 || n > 26 * 1000 {
        return n.to_string();
    }
    let letter = char::from(b'a' + ((n - 1) % 26) as u8);
    let count = (n - 1) / 26 + 1;
    let s: String = std::iter::repeat_n(letter, count as usize).collect();
    if upper { s.to_uppercase() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_encodings() {
        assert_eq!(decode_text(b"Hello"), "Hello");
        assert_eq!(decode_text(&[0xFE, 0xFF, 0x00, 0x48, 0x00, 0xE9]), "Hé");
        assert_eq!(decode_text(&[0xEF, 0xBB, 0xBF, b'o', b'k']), "ok");
        assert_eq!(decode_text(&[0x80]), "\u{2022}");
        assert_eq!(decode_text(b"a\x00b"), "ab");
    }

    #[test]
    fn encodes_round_trip() {
        for s in ["Plain", "Café", "日本", "a\u{2022}b"] {
            assert_eq!(decode_text(&encode_text(s)), s);
        }
    }

    #[test]
    fn numbering() {
        assert_eq!(roman(4, false), "iv");
        assert_eq!(roman(1994, true), "MCMXCIV");
        assert_eq!(letters(1, false), "a");
        assert_eq!(letters(28, true), "BB");
    }

    #[test]
    fn winansi() {
        assert_eq!(winansi_byte('A'), Some(b'A'));
        assert_eq!(winansi_byte('é'), Some(0xE9));
        assert_eq!(winansi_byte('€'), Some(0x80));
        assert_eq!(winansi_byte('日'), None);
    }
}
