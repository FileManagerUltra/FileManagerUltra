use fm_vfs::FmError;

/// The command language of the shell on the other end of a terminal, so paths sent to it
/// are quoted as one literal argument in that shell's rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// sh, bash, zsh, fish…
    Posix,
    /// Windows `cmd.exe`.
    Cmd,
    /// Windows PowerShell or PowerShell 7 (`pwsh`).
    PowerShell,
}

impl Dialect {
    /// The platform's default shell family.
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Cmd
        } else {
            Self::Posix
        }
    }
}

/// End, then Ctrl+Home: deletes the typed line in cmd.exe and PSReadLine (Windows mode).
const CLEAR_LINE_WINDOWS: &[u8] = b"\x1b[F\x1b[1;5H";
/// Ctrl+U: kills the typed line in readline-style shells.
const CLEAR_LINE_POSIX: &[u8] = b"\x15";

/// PowerShell treats the typographic single quotes as quote characters too.
const PS_QUOTES: [char; 5] = ['\'', '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}'];

fn check(path: &str) -> Result<(), FmError> {
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
    Ok(())
}

fn quoted(dialect: Dialect, path: &str) -> Result<String, FmError> {
    check(path)?;
    Ok(match dialect {
        Dialect::Posix => format!("'{}'", path.replace('\'', "'\\''")),
        Dialect::Cmd => {
            if path.contains(['"', '%']) {
                return Err(FmError::InvalidParams(
                    "path contains characters cmd cannot quote".to_string(),
                ));
            }
            format!("\"{path}\"")
        }
        Dialect::PowerShell => {
            let mut out = String::with_capacity(path.len() + 2);
            out.push('\'');
            for c in path.chars() {
                // Inside a single-quoted string a doubled quote is one literal quote.
                if PS_QUOTES.contains(&c) {
                    out.push(c);
                }
                out.push(c);
            }
            out.push('\'');
            out
        }
    })
}

/// The bytes that make a shell `cd` into `path`, including the Enter, with the path quoted
/// as one literal argument so a directory name can never inject commands. A leading
/// line-clear removes whatever the user had typed.
pub fn cd_command(dialect: Dialect, path: &str) -> Result<Vec<u8>, FmError> {
    let arg = quoted(dialect, path)?;
    let (clear, line) = match dialect {
        Dialect::Posix => (CLEAR_LINE_POSIX, format!("cd -- {arg}\r")),
        Dialect::Cmd => (CLEAR_LINE_WINDOWS, format!("cd /d {arg}\r")),
        Dialect::PowerShell => (
            CLEAR_LINE_WINDOWS,
            format!("Set-Location -LiteralPath {arg}\r"),
        ),
    };
    let mut out = clear.to_vec();
    out.extend_from_slice(line.as_bytes());
    Ok(out)
}

/// `path` as one literal shell word (for dropping a file onto a terminal).
pub fn path_arg(dialect: Dialect, path: &str) -> Result<String, FmError> {
    quoted(dialect, path)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn line(d: Dialect, p: &str) -> String {
        let bytes = cd_command(d, p).expect("ok");
        let clear = if d == Dialect::Posix {
            CLEAR_LINE_POSIX
        } else {
            CLEAR_LINE_WINDOWS
        };
        assert!(bytes.starts_with(clear));
        String::from_utf8_lossy(&bytes[clear.len()..]).into_owned()
    }

    #[test]
    fn quotes_spaces_and_single_quotes() {
        assert_eq!(line(Dialect::Posix, "/tmp/a b"), "cd -- '/tmp/a b'\r");
        assert_eq!(line(Dialect::Posix, "/tmp/it's"), "cd -- '/tmp/it'\\''s'\r");
        assert_eq!(line(Dialect::Cmd, r"C:\a b"), "cd /d \"C:\\a b\"\r");
        assert_eq!(
            line(Dialect::PowerShell, r"C:\it's"),
            "Set-Location -LiteralPath 'C:\\it''s'\r"
        );
    }

    #[test]
    fn hostile_names_stay_one_literal_argument() {
        assert_eq!(
            line(Dialect::Posix, "/x; rm -rf ~"),
            "cd -- '/x; rm -rf ~'\r"
        );
        assert_eq!(line(Dialect::Posix, "/x$(id)`id`"), "cd -- '/x$(id)`id`'\r");
        assert_eq!(line(Dialect::Posix, "-rf"), "cd -- '-rf'\r");
        // Single-quoted PowerShell strings expand nothing; every quote form is doubled.
        assert_eq!(
            path_arg(Dialect::PowerShell, "C:\\$(calc)`;x").expect("ok"),
            "'C:\\$(calc)`;x'"
        );
        assert_eq!(
            path_arg(Dialect::PowerShell, "a\u{2019}; calc; \u{2018}b").expect("ok"),
            "'a\u{2019}\u{2019}; calc; \u{2018}\u{2018}b'"
        );
        assert_eq!(
            line(Dialect::Cmd, r"C:\a & calc"),
            "cd /d \"C:\\a & calc\"\r"
        );
        assert!(cd_command(Dialect::Cmd, r#"C:\a" & calc & ""#).is_err());
        assert!(path_arg(Dialect::Cmd, r"C:\%PATH%").is_err());
    }

    #[test]
    fn path_args_are_single_literal_words() {
        assert_eq!(
            path_arg(Dialect::Posix, "/tmp/a b").expect("ok"),
            "'/tmp/a b'"
        );
        assert_eq!(
            path_arg(Dialect::Posix, "/it's").expect("ok"),
            "'/it'\\''s'"
        );
        assert_eq!(
            path_arg(Dialect::Posix, "/x; rm -rf ~").expect("ok"),
            "'/x; rm -rf ~'"
        );
        assert_eq!(
            path_arg(Dialect::Cmd, r"C:\a b").expect("ok"),
            "\"C:\\a b\""
        );
        for d in [Dialect::Posix, Dialect::Cmd, Dialect::PowerShell] {
            assert!(path_arg(d, "/a\nb").is_err());
            assert!(path_arg(d, "").is_err());
        }
    }

    #[test]
    fn control_characters_and_oversize_are_rejected() {
        for d in [Dialect::Posix, Dialect::Cmd, Dialect::PowerShell] {
            assert!(cd_command(d, "/a\nrm -rf ~").is_err());
            assert!(cd_command(d, "/a\x1b[31m").is_err());
            assert!(cd_command(d, "").is_err());
            assert!(cd_command(d, &"x".repeat(5000)).is_err());
        }
    }
}
