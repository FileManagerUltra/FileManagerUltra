//! OSC 52 clipboard writes (`ESC ] 52 ; c ; <base64> BEL`), as sent by programs such as
//! the Copilot CLI for "copy". Reads (`?`) are never answered, so a program cannot read
//! the host clipboard.

/// Largest base64 payload accepted (about 768 KiB of text).
const MAX_BASE64: usize = 1 << 20;

/// Collects the newest clipboard write requested by the program.
#[derive(Default)]
pub struct ClipSink {
    pending: Option<String>,
}

impl ClipSink {
    pub fn take(&mut self) -> Option<String> {
        self.pending.take()
    }
}

impl vt100::Callbacks for ClipSink {
    fn copy_to_clipboard(&mut self, _: &mut vt100::Screen, _ty: &[u8], data: &[u8]) {
        if data.len() > MAX_BASE64 {
            return;
        }
        if let Some(text) = decode(data) {
            self.pending = Some(text);
        }
    }
}

fn sextet(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode standard base64 (padding optional) into UTF-8 text; `None` when malformed,
/// empty or not valid UTF-8.
fn decode(data: &[u8]) -> Option<String> {
    let data = match data {
        [rest @ .., b'=', b'='] | [rest @ .., b'='] => rest,
        all => all,
    };
    let mut out = Vec::with_capacity(data.len() / 4 * 3 + 3);
    for chunk in data.chunks(4) {
        let mut acc = 0u32;
        for &c in chunk {
            acc = (acc << 6) | sextet(c)?;
        }
        let n = chunk.len();
        if n == 1 {
            return None;
        }
        acc <<= 6 * (4 - n) as u32;
        let bytes = acc.to_be_bytes();
        out.extend_from_slice(bytes.get(1..n)?);
    }
    let text = String::from_utf8(out).ok()?;
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_with_and_without_padding() {
        assert_eq!(decode(b"aGk=").as_deref(), Some("hi"));
        assert_eq!(decode(b"aGk").as_deref(), Some("hi"));
        assert_eq!(decode(b"aGVsbG8gd29ybGQ=").as_deref(), Some("hello world"));
        assert_eq!(decode(b"YWJj").as_deref(), Some("abc"));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"a"), None);
        assert_eq!(decode(b"****"), None);
        assert_eq!(decode(b"/w=="), None); // 0xFF is not UTF-8
    }
}
