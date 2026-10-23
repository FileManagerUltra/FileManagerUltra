//! Name validation and keep-both naming.

use fm_vfs::{FmError, MAX_COMPONENT_BYTES, Vfs, VfsUri};

/// Upper bound on `name (N).ext` probes before giving up.
pub const KEEP_BOTH_PROBE_CAP: u32 = 999;

/// A single path segment the user (or a drop payload) may hand us.
pub fn validate_name(name: &str) -> Result<(), FmError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(|c| c.is_control())
        || name.len() > MAX_COMPONENT_BYTES
    {
        return Err(FmError::InvalidParams(format!("invalid name: {name:?}")));
    }
    if cfg!(windows) && !is_windows_safe(name) {
        return Err(FmError::InvalidParams(format!("invalid name: {name:?}")));
    }
    Ok(())
}

/// Names Windows cannot store: reserved characters, device names, trailing dot/space.
pub fn is_windows_safe(name: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if name.chars().any(|c| "<>:\"|?*".contains(c)) || name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    !RESERVED.contains(&stem.as_str())
}

/// `a.txt` → `a (2).txt`; dotfiles and extension-less names get the suffix appended.
pub fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => {
            let (stem, ext) = name.split_at(dot);
            format!("{stem} ({n}){ext}")
        }
        _ => format!("{name} ({n})"),
    }
}

/// First free `name (N).ext` in `dir` (N from 2), probing with `stat`.
pub fn keep_both_name(vfs: &dyn Vfs, dir: &VfsUri, name: &str) -> Result<VfsUri, FmError> {
    for n in 2..=KEEP_BOTH_PROBE_CAP {
        let candidate = dir.join(&numbered(name, n))?;
        match vfs.stat(&candidate) {
            Err(FmError::NotFound(_)) => return Ok(candidate),
            Err(other) => return Err(other),
            Ok(_) => continue,
        }
    }
    Err(FmError::InvalidParams(format!(
        "no free name for {name:?} after {KEEP_BOTH_PROBE_CAP} tries"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;

    #[test]
    fn rejects_bad_names() {
        for bad in ["", ".", "..", "a/b", "a\\b", "a\nb", &"x".repeat(5000)] {
            assert!(
                matches!(validate_name(bad), Err(FmError::InvalidParams(_))),
                "{bad:?}"
            );
        }
        assert!(validate_name("ok name.txt").is_ok());
    }

    #[test]
    fn windows_rules_are_checked_by_the_helper() {
        for bad in ["a:b", "a?", "CON", "nul.txt", "x.", "x ", "a|b"] {
            assert!(!is_windows_safe(bad), "{bad:?}");
        }
        assert!(is_windows_safe("console.txt"));
        assert!(is_windows_safe("ok name.txt"));
    }

    #[test]
    fn numbering_handles_extensions_and_dotfiles() {
        assert_eq!(numbered("a.txt", 2), "a (2).txt");
        assert_eq!(numbered("a.tar.gz", 3), "a.tar (3).gz");
        assert_eq!(numbered(".bashrc", 2), ".bashrc (2)");
        assert_eq!(numbered("README", 2), "README (2)");
    }

    #[test]
    fn keep_both_picks_next_free() {
        let fs = MemFs::new();
        fs.put_str("/d/a.txt", b"1").expect("put");
        fs.put_str("/d/a (2).txt", b"2").expect("put");
        let dir = VfsUri::parse("/d").expect("uri");
        let got = keep_both_name(&fs, &dir, "a.txt").expect("name");
        assert_eq!(got.file_name(), Some("a (3).txt"));
    }
}
