//! Which program a terminal runs, and the command language it speaks.

use std::ffi::OsString;
use std::path::PathBuf;

use fm_vfs::FmError;
use portable_pty::CommandBuilder;

use crate::quote::Dialect;

/// The user's shell choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shell {
    /// PowerShell on Windows (falling back to cmd), the login shell elsewhere.
    #[default]
    Default,
    /// PowerShell 7 (`pwsh`) when installed, else Windows PowerShell.
    PowerShell,
    /// `cmd.exe` (Windows only).
    Cmd,
}

/// A resolved program to start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Program {
    /// The platform default (`$SHELL` / `%COMSPEC%`).
    Native,
    Path {
        exe: PathBuf,
        args: &'static [&'static str],
    },
}

const PS_ARGS: &[&str] = &["-NoLogo"];

/// First `name` found in the `PATH`-style list `path`.
fn search(path: Option<OsString>, name: &str) -> Option<PathBuf> {
    std::env::split_paths(&path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn powershell(path: Option<OsString>, system_root: Option<OsString>) -> Option<PathBuf> {
    if cfg!(windows) {
        search(path.clone(), "pwsh.exe")
            .or_else(|| {
                let root = PathBuf::from(system_root?);
                let exe = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
                exe.is_file().then_some(exe)
            })
            .or_else(|| search(path, "powershell.exe"))
    } else {
        search(path, "pwsh")
    }
}

fn cmd(comspec: Option<OsString>, system_root: Option<OsString>) -> Option<PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    comspec
        .map(PathBuf::from)
        .filter(|p| p.is_absolute() && p.is_file())
        .or_else(|| {
            let exe = PathBuf::from(system_root?).join(r"System32\cmd.exe");
            exe.is_file().then_some(exe)
        })
}

impl Shell {
    pub(crate) fn resolve(self) -> Result<(Program, Dialect), FmError> {
        let path = || std::env::var_os("PATH");
        let root = || std::env::var_os("SystemRoot");
        match self {
            Shell::Default if cfg!(windows) => Ok(powershell(path(), root())
                .map_or((Program::Native, Dialect::Cmd), |exe| {
                    (Program::Path { exe, args: PS_ARGS }, Dialect::PowerShell)
                })),
            Shell::Default => Ok((Program::Native, Dialect::native())),
            Shell::PowerShell => powershell(path(), root())
                .map(|exe| (Program::Path { exe, args: PS_ARGS }, Dialect::PowerShell))
                .ok_or(FmError::Unsupported("PowerShell is not installed")),
            Shell::Cmd => cmd(std::env::var_os("COMSPEC"), root())
                .map(|exe| (Program::Path { exe, args: &[] }, Dialect::Cmd))
                .ok_or(FmError::Unsupported("cmd.exe is only available on Windows")),
        }
    }
}

impl Program {
    pub(crate) fn command(&self) -> CommandBuilder {
        match self {
            Program::Native => CommandBuilder::new_default_prog(),
            Program::Path { exe, args } => {
                let mut cmd = CommandBuilder::new(exe.as_os_str());
                cmd.args(args.iter());
                cmd
            }
        }
    }

    #[cfg(all(test, unix))]
    pub(crate) fn exe(&self) -> Option<&std::path::Path> {
        match self {
            Program::Native => None,
            Program::Path { exe, .. } => Some(exe),
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::expect_used)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn powershell_is_found_on_the_path_only_as_an_absolute_file() {
        let dir = tempfile::tempdir().expect("tmp");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).expect("mkdir");
        let list = |dirs: &[&Path]| std::env::join_paths(dirs).ok();
        assert_eq!(powershell(list(&[&bin]), None), None);
        std::fs::write(bin.join("pwsh"), "").expect("write");
        assert_eq!(powershell(list(&[&bin]), None), Some(bin.join("pwsh")));
        // Relative PATH entries are never searched.
        assert_eq!(powershell(list(&[Path::new("bin")]), None), None);
        assert_eq!(powershell(None, None), None);
    }

    #[test]
    fn cmd_does_not_exist_here_and_default_is_the_login_shell() {
        assert_eq!(cmd(Some("/bin/sh".into()), None), None);
        assert!(Shell::Cmd.resolve().is_err());
        let (program, dialect) = Shell::Default.resolve().expect("default");
        assert_eq!(program.exe(), None);
        assert_eq!(dialect, Dialect::Posix);
    }
}
