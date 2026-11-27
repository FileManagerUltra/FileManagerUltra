//! Encodings and line endings: detect on load, decode in chunks, encode on save so a
//! file round-trips byte for byte unless the user changes it.

use std::fmt;

/// How many leading bytes are sniffed.
pub const SNIFF_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    /// The fallback for 8-bit text that is not UTF-8; every byte is one char.
    Latin1,
}

impl Encoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Utf16Le => "UTF-16 LE",
            Self::Utf16Be => "UTF-16 BE",
            Self::Latin1 => "Latin-1",
        }
    }

    pub fn bom(self) -> &'static [u8] {
        match self {
            Self::Utf8Bom => &[0xEF, 0xBB, 0xBF],
            Self::Utf16Le => &[0xFF, 0xFE],
            Self::Utf16Be => &[0xFE, 0xFF],
            Self::Utf8 | Self::Latin1 => &[],
        }
    }

    fn is_wide(self) -> bool {
        matches!(self, Self::Utf16Le | Self::Utf16Be)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Eol {
    Lf,
    CrLf,
}

impl Eol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::CrLf => "CRLF",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Detected {
    pub encoding: Encoding,
    /// Looks like binary data, not text: the editor opens read-only.
    pub binary: bool,
}

/// Looks at the start of a file. `sample` should be at most `SNIFF_BYTES` long, and
/// `whole_file` says it is the entire file (so a cut-off character is an error, not
/// just the edge of the sample).
pub fn sniff(sample: &[u8], whole_file: bool) -> Detected {
    let text = |encoding| Detected {
        encoding,
        binary: false,
    };
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return text(Encoding::Utf8Bom);
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return text(Encoding::Utf16Le);
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return text(Encoding::Utf16Be);
    }
    if sample.contains(&0) {
        return Detected {
            encoding: Encoding::Utf8,
            binary: true,
        };
    }
    match std::str::from_utf8(sample) {
        Ok(_) => text(Encoding::Utf8),
        // A character cut off by the end of the sample is still valid UTF-8.
        Err(e) if !whole_file && e.error_len().is_none() => text(Encoding::Utf8),
        Err(_) => text(Encoding::Latin1),
    }
}

/// The dominant line ending of `text` and whether the other kind also occurs.
pub fn detect_eol(text: &str) -> (Eol, bool) {
    let mut crlf = 0usize;
    let mut lf = 0usize;
    let mut prev = '\0';
    for c in text.chars() {
        if c == '\n' {
            if prev == '\r' {
                crlf += 1;
            } else {
                lf += 1;
            }
        }
        prev = c;
    }
    let eol = if crlf > lf { Eol::CrLf } else { Eol::Lf };
    (eol, crlf > 0 && lf > 0)
}

/// Rewrites every line ending of `text` to `eol`.
pub fn convert_eol(text: &str, eol: Eol) -> String {
    let lf = text.replace("\r\n", "\n");
    match eol {
        Eol::Lf => lf,
        Eol::CrLf => lf.replace('\n', "\r\n"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The bytes are not valid in the chosen encoding; try `Encoding::Latin1`.
    Invalid,
    /// The data ended inside a character.
    Truncated,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid => f.write_str("the file is not valid text in this encoding"),
            Self::Truncated => f.write_str("the file ends in the middle of a character"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Chunked decoder: bytes in any split, text out, BOM stripped.
#[derive(Debug)]
pub struct Decoder {
    encoding: Encoding,
    pending: Vec<u8>,
    bom_left: usize,
}

impl Decoder {
    /// For the encodings with a BOM the file is expected to start with it; it is skipped.
    pub fn new(encoding: Encoding) -> Self {
        Self {
            encoding,
            pending: Vec::new(),
            bom_left: encoding.bom().len(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<String, DecodeError> {
        self.pending.extend_from_slice(bytes);
        if self.bom_left > 0 {
            let n = self.bom_left.min(self.pending.len());
            self.pending.drain(..n);
            self.bom_left -= n;
        }
        let data = std::mem::take(&mut self.pending);
        match self.encoding {
            Encoding::Utf8 | Encoding::Utf8Bom => self.push_utf8(data),
            Encoding::Utf16Le | Encoding::Utf16Be => self.push_utf16(data),
            Encoding::Latin1 => Ok(data.iter().map(|&b| char::from(b)).collect()),
        }
    }

    /// Ends the stream; leftover bytes mean the file was cut mid-character.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(DecodeError::Truncated)
        }
    }

    fn push_utf8(&mut self, data: Vec<u8>) -> Result<String, DecodeError> {
        match String::from_utf8(data) {
            Ok(s) => Ok(s),
            Err(e) => {
                let err = e.utf8_error();
                if err.error_len().is_some() {
                    return Err(DecodeError::Invalid);
                }
                let valid = err.valid_up_to();
                let mut bytes = e.into_bytes();
                self.pending = bytes.split_off(valid);
                String::from_utf8(bytes).map_err(|_| DecodeError::Invalid)
            }
        }
    }

    fn push_utf16(&mut self, data: Vec<u8>) -> Result<String, DecodeError> {
        let big = self.encoding == Encoding::Utf16Be;
        let mut units: Vec<u16> = data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&p| {
                if big {
                    u16::from_be_bytes(p)
                } else {
                    u16::from_le_bytes(p)
                }
            })
            .collect();
        let even = data.len() - data.len() % 2;
        let mut pending = data.get(even..).unwrap_or_default().to_vec();
        // A high surrogate at the end waits for its partner in the next chunk.
        if let Some(&last) = units.last()
            && (0xD800..0xDC00).contains(&last)
        {
            units.pop();
            let b = if big {
                last.to_be_bytes()
            } else {
                last.to_le_bytes()
            };
            let mut held = b.to_vec();
            held.append(&mut pending);
            pending = held;
        }
        self.pending = pending;
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|_| DecodeError::Invalid)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// A char that the target encoding cannot hold (Latin-1 only).
    Unrepresentable(char),
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unrepresentable(c) => {
                write!(f, "the character {c:?} cannot be saved in this encoding")
            }
        }
    }
}

impl std::error::Error for EncodeError {}

/// Appends `text` encoded to `out`. Call once per rope chunk; write the BOM first.
pub fn encode_into(enc: Encoding, text: &str, out: &mut Vec<u8>) -> Result<(), EncodeError> {
    match enc {
        Encoding::Utf8 | Encoding::Utf8Bom => out.extend_from_slice(text.as_bytes()),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            for unit in text.encode_utf16() {
                let b = if enc == Encoding::Utf16Be {
                    unit.to_be_bytes()
                } else {
                    unit.to_le_bytes()
                };
                out.extend_from_slice(&b);
            }
        }
        Encoding::Latin1 => {
            for c in text.chars() {
                let b = u8::try_from(u32::from(c)).map_err(|_| EncodeError::Unrepresentable(c))?;
                out.push(b);
            }
        }
    }
    Ok(())
}

impl Encoding {
    /// True for the encodings that always carry a BOM when written.
    pub fn writes_bom(self) -> bool {
        self.is_wide() || self == Self::Utf8Bom
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(enc: Encoding, bytes: &[u8], split: usize) -> Result<String, DecodeError> {
        let mut d = Decoder::new(enc);
        let mut out = String::new();
        for chunk in bytes.chunks(split.max(1)) {
            out.push_str(&d.push(chunk)?);
        }
        d.finish()?;
        Ok(out)
    }

    fn encode_all(enc: Encoding, text: &str) -> Vec<u8> {
        let mut out = enc.bom().to_vec();
        encode_into(enc, text, &mut out).expect("encode");
        out
    }

    #[test]
    fn sniffs_boms_binary_and_fallback() {
        assert_eq!(sniff(b"\xEF\xBB\xBFhi", true).encoding, Encoding::Utf8Bom);
        assert_eq!(sniff(b"\xFF\xFEh\0", true).encoding, Encoding::Utf16Le);
        assert_eq!(sniff(b"\xFE\xFF\0h", true).encoding, Encoding::Utf16Be);
        assert!(sniff(b"ab\0cd", true).binary);
        assert_eq!(sniff("héllo".as_bytes(), true).encoding, Encoding::Utf8);
        assert_eq!(sniff(b"caf\xE9", true).encoding, Encoding::Latin1);
        // A cut-off multi-byte char at the end of the sample is fine.
        let cut = "é".as_bytes().get(..1).expect("byte");
        assert_eq!(sniff(cut, false).encoding, Encoding::Utf8);
        assert_eq!(sniff(cut, true).encoding, Encoding::Latin1);
    }

    #[test]
    fn eol_detection_and_conversion() {
        assert_eq!(detect_eol("a\nb\n"), (Eol::Lf, false));
        assert_eq!(detect_eol("a\r\nb\r\n"), (Eol::CrLf, false));
        assert_eq!(detect_eol("a\r\nb\r\nc\n"), (Eol::CrLf, true));
        assert_eq!(detect_eol("no newline"), (Eol::Lf, false));
        assert_eq!(convert_eol("a\r\nb\nc", Eol::CrLf), "a\r\nb\r\nc");
        assert_eq!(convert_eol("a\r\nb\nc", Eol::Lf), "a\nb\nc");
    }

    #[test]
    fn round_trips_every_encoding_in_awkward_chunks() {
        let text = "héllo wörld — 日本語 😀\r\nline two\n";
        for enc in [
            Encoding::Utf8,
            Encoding::Utf8Bom,
            Encoding::Utf16Le,
            Encoding::Utf16Be,
        ] {
            let bytes = encode_all(enc, text);
            for split in [1, 2, 3, 5, 7, 4096] {
                assert_eq!(decode_all(enc, &bytes, split).as_deref(), Ok(text));
            }
        }
        let latin = "café ñ ÿ";
        let bytes = encode_all(Encoding::Latin1, latin);
        assert_eq!(
            decode_all(Encoding::Latin1, &bytes, 3).as_deref(),
            Ok(latin)
        );
    }

    #[test]
    fn invalid_utf8_is_reported_so_the_loader_can_retry_as_latin1() {
        assert_eq!(
            decode_all(Encoding::Utf8, b"ok \xFF bad", 4),
            Err(DecodeError::Invalid)
        );
    }

    #[test]
    fn truncated_input_is_reported() {
        let cut = "日".as_bytes().get(..2).expect("bytes");
        assert_eq!(
            decode_all(Encoding::Utf8, cut, 8),
            Err(DecodeError::Truncated)
        );
        assert_eq!(
            decode_all(Encoding::Utf16Le, &[0xFF, 0xFE, b'a'], 8),
            Err(DecodeError::Truncated)
        );
    }

    #[test]
    fn unpaired_surrogate_is_invalid() {
        assert_eq!(
            decode_all(Encoding::Utf16Le, &[0xFF, 0xFE, 0x00, 0xD8, b'a', 0], 16),
            Err(DecodeError::Invalid)
        );
    }

    #[test]
    fn latin1_rejects_wider_chars() {
        let mut out = Vec::new();
        assert_eq!(
            encode_into(Encoding::Latin1, "a日", &mut out),
            Err(EncodeError::Unrepresentable('日'))
        );
    }

    #[test]
    fn bom_is_stripped_when_it_arrives_in_pieces() {
        assert_eq!(
            decode_all(Encoding::Utf8Bom, b"\xEF\xBB\xBFhi", 1).as_deref(),
            Ok("hi")
        );
    }
}
