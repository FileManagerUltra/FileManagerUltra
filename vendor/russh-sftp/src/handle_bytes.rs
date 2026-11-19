//! Local patch (see `vendor/FM-VENDORED.md` in the workspace).
//!
//! SFTP handles are opaque bytes chosen by the server, but this release keeps them as
//! `String` and decoded them as lossy UTF-8. A server whose handles contain bytes of 0x80 or
//! more (asyncssh uses a 4-byte big-endian counter, so handle 128 is `00 00 00 80`) got back a
//! different handle and answered "Invalid file handle"; 128 out of every 256 files failed.
//!
//! Handle fields are therefore mapped one byte per `char` (Latin-1): every byte sequence
//! round-trips exactly. A handle string made elsewhere with a character above U+00FF is
//! written as that character's UTF-8 bytes, which then also round-trip unchanged.

use serde::de::{self, Deserializer, Visitor};
use serde::Serializer;

/// The wire bytes of a handle kept as a `String`.
pub fn to_bytes(handle: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(handle.len());
    for c in handle.chars() {
        match u8::try_from(u32::from(c)) {
            Ok(b) => out.push(b),
            Err(_) => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out
}

/// A handle's wire bytes as a `String`, one `char` per byte.
pub fn from_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

pub fn serialize<S: Serializer, T: AsRef<str>>(handle: &T, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_bytes(&to_bytes(handle.as_ref()))
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    struct HandleVisitor;

    impl Visitor<'_> for HandleVisitor {
        type Value = String;

        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an SFTP handle")
        }

        fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<String, E> {
            Ok(from_bytes(v))
        }

        fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<String, E> {
            Ok(from_bytes(&v))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(from_bytes(v.as_bytes()))
        }
    }

    d.deserialize_byte_buf(HandleVisitor)
}

