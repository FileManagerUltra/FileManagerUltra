//! Credential redaction (capability `security`).
//!
//! `FmError::Display` runs every attacker- or user-supplied string through
//! [`redact`] so `user:password@host` shapes and `key=value` secrets never
//! reach a toast, a log line or a screenshot. The scanner is byte-oriented
//! and total: it never panics on arbitrary input.

/// Secrets we mask when they appear as `key=value` / `key: value`.
const SECRET_KEYS: &[&[u8]] = &[
    b"password",
    b"passwd",
    b"pwd",
    b"pass",
    b"token",
    b"secret",
    b"access_key",
    b"api_key",
];

const MASK: &[u8] = b"***";

/// Replace credential-shaped substrings with `***`; everything else is preserved.
pub fn redact(input: &str) -> String {
    let mut bytes = input.as_bytes().to_vec();
    mask_uri_userinfo(&mut bytes);
    mask_key_values(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'&' | b';' | b',' | b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\''
    )
}

/// `scheme://user:password@host` → mask the password between `:` and `@`.
fn mask_uri_userinfo(buf: &mut [u8]) {
    let n = buf.len();
    let mut i = 0;
    while i + 3 <= n {
        if &buf[i..i + 3] == b"://" {
            // Authority spans from i+3 to the first delimiter.
            let start = i + 3;
            let mut end = start;
            while end < n && !matches!(buf[end], b'/' | b'?' | b'#') && !is_delim(buf[end]) {
                end += 1;
            }
            // Find the *last* '@' in the authority (userinfo may itself contain '@').
            if let Some(at) = (start..end).rev().find(|&j| buf[j] == b'@') {
                // Find the *first* ':' before '@' — that starts the password.
                if let Some(colon) = (start..at).find(|&j| buf[j] == b':') {
                    mask_range(buf, colon + 1, at);
                }
            } else if let Some(colon) = (start..end).find(|&j| buf[j] == b':') {
                // `scheme://user:password` with no host yet — but `host:2222` is a port.
                let after = &buf[colon + 1..end];
                if !after.is_empty() && !after.iter().all(|b| b.is_ascii_digit()) {
                    mask_range(buf, colon + 1, end);
                }
            }
            i = end.max(i + 1);
        } else {
            i += 1;
        }
    }
    mask_bare_userinfo(buf);
}

/// `bob:s3cret@nas` (no scheme) → mask between `:` and `@`. A colon separated from
/// the `@` by whitespace (`note: see @home`) is not userinfo.
fn mask_bare_userinfo(buf: &mut [u8]) {
    let n = buf.len();
    let mut at = 0;
    while at < n {
        if buf[at] == b'@' {
            let mut lo = at;
            while lo > 0 && !is_delim(buf[lo - 1]) && !matches!(buf[lo - 1], b'/' | b'?' | b'#') {
                lo -= 1;
            }
            if let Some(colon) = (lo..at).find(|&j| buf[j] == b':') {
                mask_range(buf, colon + 1, at);
            }
        }
        at += 1;
    }
}

fn mask_range(buf: &mut [u8], from: usize, to: usize) {
    if from >= to {
        return;
    }
    let keep = MASK.len().min(to - from);
    buf[from..from + keep].copy_from_slice(&MASK[..keep]);
    for b in &mut buf[from + keep..to] {
        *b = b'*';
    }
}

/// `password=hunter2`, `token: abc`, … → mask the value up to a delimiter.
fn mask_key_values(buf: &mut [u8]) {
    let n = buf.len();
    for key in SECRET_KEYS {
        let klen = key.len();
        if n < klen + 1 {
            continue;
        }
        let mut start = 0;
        while start + klen <= n {
            if matches_ascii_ci_eq(&buf[start..start + klen], key) {
                let mut j = start + klen;
                // Optional whitespace around the separator.
                while j < n && matches!(buf[j], b' ' | b'\t') {
                    j += 1;
                }
                if j < n && matches!(buf[j], b'=' | b':') {
                    j += 1;
                    while j < n && matches!(buf[j], b' ' | b'\t') {
                        j += 1;
                    }
                    let vstart = j;
                    while j < n && !is_delim(buf[j]) {
                        j += 1;
                    }
                    if j > vstart {
                        let keep = MASK.len().min(j - vstart);
                        buf[vstart..vstart + keep].copy_from_slice(&MASK[..keep]);
                        for b in &mut buf[vstart + keep..j] {
                            *b = b'*';
                        }
                    }
                    start = j;
                } else {
                    start += klen;
                }
            } else {
                start += 1;
            }
        }
    }
}

fn matches_ascii_ci_eq(hay: &[u8], needle: &[u8]) -> bool {
    hay.len() == needle.len()
        && hay
            .iter()
            .zip(needle.iter())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn masks_password_in_url() {
        let out = redact("sftp://alice:hunter2@example.com/x");
        assert!(!out.contains("hunter2"), "{out}");
        assert!(out.contains("alice"));
        assert!(out.contains("example.com"));
    }

    #[test]
    fn masks_password_in_url_with_port() {
        let out = redact("smb://bob:p@ss@nas:445/share");
        assert!(!out.contains("p@ss"), "{out}");
        assert!(out.contains("nas:445"), "{out}");
    }

    #[test]
    fn masks_key_value_forms() {
        for input in [
            "login failed: password=hunter2",
            "retrying with PASSWD : hunter2",
            "token: abc123 next",
            "api_key=j5k6j7",
        ] {
            let out = redact(input);
            assert!(
                !out.contains("hunter2") && !out.contains("abc123") && !out.contains("j5k6j7"),
                "{input} → {out}"
            );
        }
    }

    #[test]
    fn leaves_plain_text_alone() {
        assert_eq!(
            redact("directory not found: /home/alice"),
            "directory not found: /home/alice"
        );
    }

    #[test]
    fn password_at_end_of_string() {
        let out = redact("sftp://alice:hunter2");
        assert!(!out.contains("hunter2"), "{out}");
    }

    #[test]
    fn bare_userinfo_without_scheme() {
        let out = redact("auth failed for bob:s3cret@nas");
        assert!(!out.contains("s3cret"), "{out}");
        assert!(out.contains("bob") && out.contains("nas"), "{out}");
    }

    #[test]
    fn port_is_not_a_password() {
        let out = redact("sftp://host:2222/x");
        assert!(out.contains("2222"), "{out}");
    }

    #[test]
    fn colon_far_from_at_is_not_userinfo() {
        let out = redact("note: see @home");
        assert_eq!(out, "note: see @home");
    }

    #[test]
    fn word_password_as_filename_is_kept() {
        // "password" as a bare word with no separator is not a secret assignment.
        let out = redact("/home/alice/password");
        assert!(out.contains("password"), "{out}");
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_text(s in "\\PC{0,512}") {
            let out = redact(&s);
            prop_assert!(out.len() <= s.len().saturating_add(16));
        }

        #[test]
        fn never_panics_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let s = String::from_utf8_lossy(&bytes);
            let out = redact(&s);
            prop_assert!(out.len() <= s.len().saturating_add(16));
        }

        #[test]
        fn no_separators_means_unchanged(s in "[^:@]{1,64}") {
            // Without ':' or '@' there is no userinfo and no key=value assignment.
            prop_assert_eq!(redact(&s), s);
        }
    }
}
