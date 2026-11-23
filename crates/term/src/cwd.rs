//! The shell's current folder as the shell reports it. Where the OS does not show another
//! process's working directory without FFI (Windows, macOS), a shell can say where it is:
//! OSC 7 (`ESC ] 7 ; file://host/path ST`, the common convention) or OSC 9;9
//! (`ESC ] 9 ; 9 ; "C:\path" ST`, Windows Terminal's). Windows shells are set up to send
//! OSC 9;9 from their prompt ([`powershell_hook`], [`cmd_prompt`]).
//!
//! The payload is untrusted program output: it is bounded, must be UTF-8 without control
//! characters and must be an absolute path. Whether the folder exists is checked by the
//! caller before it is used.

/// Longest reported path accepted, in bytes.
const MAX_PATH: usize = 4096;

/// The folder an OSC sequence reports, if it is OSC 7 or OSC 9;9 with a sane path.
/// `params` are the sequence's `;`-separated fields, as vt100 hands them over.
pub(crate) fn from_osc(params: &[&[u8]]) -> Option<String> {
    let rest = |from: usize| -> Option<String> {
        let parts = params.get(from..)?;
        if parts.is_empty() || parts.iter().map(|p| p.len() + 1).sum::<usize>() > MAX_PATH * 3 {
            return None;
        }
        // A path may itself contain `;`: put the fields back together.
        let joined = parts.join(&b';');
        String::from_utf8(joined).ok()
    };
    let path = match params {
        [b"7", ..] => from_file_url(&rest(1)?)?,
        [b"9", b"9", ..] => {
            let text = rest(2)?;
            let text = text.trim();
            text.strip_prefix('"')
                .and_then(|t| t.strip_suffix('"'))
                .unwrap_or(text)
                .to_string()
        }
        _ => return None,
    };
    sane(path)
}

/// `file://host/path` → the local path (percent-decoded). The host is not checked: a
/// remote shell's folder will not exist here, which the caller tests.
fn from_file_url(url: &str) -> Option<String> {
    let after = url.strip_prefix("file://")?;
    let slash = after.find('/')?;
    let encoded = after.get(slash..)?;
    let bytes = percent_decode(encoded)?;
    let path = String::from_utf8(bytes).ok()?;
    // `file:///C:/Users/x` names `C:/Users/x` on Windows.
    let windows_drive = path
        .strip_prefix('/')
        .filter(|p| drive_prefix(p))
        .map(str::to_string);
    Some(windows_drive.unwrap_or(path))
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            out.push(u8::try_from(hi * 16 + lo).ok()?);
        } else {
            out.push(b);
        }
    }
    Some(out)
}

/// `C:\…` or `C:/…`.
fn drive_prefix(path: &str) -> bool {
    let mut chars = path.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(d), Some(':'), Some('\\' | '/')) if d.is_ascii_alphabetic()
    )
}

fn sane(path: String) -> Option<String> {
    let absolute = path.starts_with('/') || path.starts_with(r"\\") || drive_prefix(&path);
    (absolute && path.len() <= MAX_PATH && !path.chars().any(char::is_control)).then_some(path)
}

/// PowerShell startup command (for `-EncodedCommand`): wrap whatever prompt the profile set
/// so each prompt first reports the current folder with OSC 9;9, as Windows Terminal's
/// documentation suggests. Only file-system locations are reported.
pub(crate) const POWERSHELL_HOOK: &str = concat!(
    "$global:__fmPrompt = $function:prompt\n",
    "function global:prompt {\n",
    "  $loc = $executionContext.SessionState.Path.CurrentLocation\n",
    "  $out = ''\n",
    "  if ($loc.Provider.Name -eq 'FileSystem') {\n",
    "    $out = \"$([char]27)]9;9;`\"$($loc.ProviderPath)`\"$([char]27)\\\"\n",
    "  }\n",
    "  $out + (& $global:__fmPrompt)\n",
    "}\n",
);

/// `POWERSHELL_HOOK` as PowerShell's `-EncodedCommand` wants it: base64 of UTF-16LE. No
/// quoting can go wrong on the command line this way.
pub(crate) fn powershell_hook() -> String {
    let utf16: Vec<u8> = POWERSHELL_HOOK
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    base64(&utf16)
}

/// cmd's `PROMPT` with the OSC 9;9 report (`$e]9;9;$P$e\`) in front of the user's own prompt
/// (`$P$G`, cmd's default, when none is set).
pub(crate) fn cmd_prompt(current: Option<&str>) -> String {
    let own = current.filter(|p| !p.is_empty()).unwrap_or("$P$G");
    format!("$e]9;9;$P$e\\{own}")
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                let ix = ((n >> shift) & 63) as usize;
                out.push(char::from(TABLE.get(ix).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn osc(text: &str) -> Option<String> {
        let parts: Vec<&[u8]> = text.as_bytes().split(|b| *b == b';').collect();
        from_osc(&parts)
    }

    #[test]
    fn osc7_file_urls() {
        assert_eq!(osc("7;file://host/home/me").as_deref(), Some("/home/me"));
        assert_eq!(osc("7;file:///tmp/a%20b").as_deref(), Some("/tmp/a b"));
        assert_eq!(osc("7;file:///C:/Users/me").as_deref(), Some("C:/Users/me"));
        assert_eq!(osc("7;file://h/odd;name").as_deref(), Some("/odd;name"));
        for bad in [
            "7;http://host/x",
            "7;file://hostonly",
            "7;file:///bad%zz",
            "7;file:///a%0Ab",
            "7;",
            "7",
        ] {
            assert_eq!(osc(bad), None, "{bad}");
        }
    }

    #[test]
    fn osc9_9_windows_paths() {
        assert_eq!(osc(r#"9;9;"C:\Users\me""#).as_deref(), Some(r"C:\Users\me"));
        assert_eq!(osc(r"9;9;D:\work").as_deref(), Some(r"D:\work"));
        assert_eq!(
            osc(r#"9;9;"\\server\share\x""#).as_deref(),
            Some(r"\\server\share\x")
        );
        assert_eq!(osc(r#"9;9;"C:\a;b""#).as_deref(), Some(r"C:\a;b"));
        for bad in [
            r"9;9;relative\dir",
            "9;9;",
            "9;4;1;50",
            "0;title",
            "52;c;aGk=",
        ] {
            assert_eq!(osc(bad), None, "{bad}");
        }
        let long = format!("9;9;C:\\{}", "a".repeat(MAX_PATH + 10));
        assert_eq!(osc(&long), None, "oversized");
    }

    #[test]
    fn cmd_prompt_keeps_the_users_prompt() {
        assert_eq!(cmd_prompt(None), r"$e]9;9;$P$e\$P$G");
        assert_eq!(cmd_prompt(Some("")), r"$e]9;9;$P$e\$P$G");
        assert_eq!(cmd_prompt(Some("$T $P$G")), r"$e]9;9;$P$e\$T $P$G");
    }

    #[test]
    fn powershell_hook_is_utf16le_base64() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"abc"), "YWJj");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
        let encoded = powershell_hook();
        // Decoding it back gives the script.
        let decoded = crate::clip::decode_bytes(encoded.as_bytes()).expect("base64");
        let units: Vec<u16> = decoded
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(String::from_utf16(&units).expect("utf16"), POWERSHELL_HOOK);
    }
}
