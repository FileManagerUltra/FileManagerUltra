use fm_vfs::FmError;

/// The bytes that make a shell `cd` into `path`, including the Enter, with the path quoted
/// as one literal argument so a directory name can never inject commands. A leading Ctrl+U
/// clears whatever the user had typed on the line.
pub fn cd_command(path: &str) -> Result<Vec<u8>, FmError> {
    if path.is_empty() || path.len() > 4096 {
        return Err(FmError::InvalidParams(
            "path must be 1..=4096 bytes".to_string(),
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(FmError::InvalidParams(
            "path contains control characters".to_string(),
        ));
    }
    let line = if cfg!(windows) {
        if path.contains(['"', '%']) {
            return Err(FmError::InvalidParams(
                "path contains characters cmd cannot quote".to_string(),
            ));
        }
        format!("cd /d \"{path}\"\r")
    } else {
        format!("cd -- '{}'\r", path.replace('\'', "'\\''"))
    };
    let mut out = vec![0x15];
    out.extend_from_slice(line.as_bytes());
    Ok(out)
}

/// `path` as one literal shell word (for dropping a file onto a terminal).
pub fn path_arg(path: &str) -> Result<String, FmError> {
    if path.is_empty() || path.len() > 4096 {
        return Err(FmError::InvalidParams(
            "path must be 1..=4096 bytes".to_string(),
        ));
    }
    if path.chars().any(char::is_control) {
        return Err(FmError::InvalidParams(
            "path contains control characters".to_string(),
        ));
    }
    if cfg!(windows) {
        if path.contains(['"', '%']) {
            return Err(FmError::InvalidParams(
                "path contains characters cmd cannot quote".to_string(),
            ));
        }
        Ok(format!("\"{path}\""))
    } else {
        Ok(format!("'{}'", path.replace('\'', "'\\''")))
    }
}

#[cfg(all(test, not(windows)))]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn line(p: &str) -> String {
        String::from_utf8_lossy(&cd_command(p).expect("ok")[1..]).into_owned()
    }

    #[test]
    fn quotes_spaces_and_single_quotes() {
        assert_eq!(line("/tmp/a b"), "cd -- '/tmp/a b'\r");
        assert_eq!(line("/tmp/it's"), "cd -- '/tmp/it'\\''s'\r");
    }

    #[test]
    fn hostile_names_stay_one_literal_argument() {
        assert_eq!(line("/x; rm -rf ~"), "cd -- '/x; rm -rf ~'\r");
        assert_eq!(line("/x$(id)`id`"), "cd -- '/x$(id)`id`'\r");
        assert_eq!(line("-rf"), "cd -- '-rf'\r");
    }

    #[test]
    fn path_args_are_single_literal_words() {
        assert_eq!(path_arg("/tmp/a b").expect("ok"), "'/tmp/a b'");
        assert_eq!(path_arg("/it's").expect("ok"), "'/it'\\''s'");
        assert_eq!(path_arg("/x; rm -rf ~").expect("ok"), "'/x; rm -rf ~'");
        assert!(path_arg("/a\nb").is_err());
        assert!(path_arg("").is_err());
    }

    #[test]
    fn control_characters_and_oversize_are_rejected() {
        assert!(cd_command("/a\nrm -rf ~").is_err());
        assert!(cd_command("/a\x1b[31m").is_err());
        assert!(cd_command("").is_err());
        assert!(cd_command(&"x".repeat(5000)).is_err());
    }
}
