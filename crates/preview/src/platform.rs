//! Per-OS behaviour isolated behind pure functions so the Linux test run covers all
//! three branches. Used for locating the optional video decoder.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    MacOs,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }

    pub fn exe_name(self, tool: &str) -> String {
        match self {
            Os::Windows => format!("{tool}.exe"),
            _ => tool.to_string(),
        }
    }

    fn path_sep(self) -> char {
        if self == Os::Windows { ';' } else { ':' }
    }
}

/// Where to look for `tool`, best first: a sidecar next to the executable, then every
/// `PATH` entry, then the usual package-manager locations (GUI apps on macOS get a
/// minimal `PATH`).
pub fn tool_candidates(os: Os, tool: &str, exe_dir: Option<&Path>, path_env: &str) -> Vec<PathBuf> {
    let name = os.exe_name(tool);
    let mut out = Vec::new();
    if let Some(d) = exe_dir {
        out.push(d.join(&name));
        out.push(d.join("bin").join(&name));
    }
    out.extend(
        path_env
            .split(os.path_sep())
            .filter(|p| !p.is_empty())
            .map(|p| Path::new(p).join(&name)),
    );
    if os == Os::MacOs {
        for d in ["/opt/homebrew/bin", "/usr/local/bin"] {
            out.push(Path::new(d).join(&name));
        }
    }
    let mut seen = Vec::new();
    out.retain(|p| {
        let fresh = !seen.contains(p);
        seen.push(p.clone());
        fresh
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_first_then_path_per_os() {
        let l = tool_candidates(
            Os::Linux,
            "ffmpeg",
            Some(Path::new("/opt/app")),
            "/usr/bin:/bin",
        );
        assert_eq!(l[0], Path::new("/opt/app/ffmpeg"));
        assert!(l.contains(&PathBuf::from("/usr/bin/ffmpeg")));
        let w = tool_candidates(
            Os::Windows,
            "ffmpeg",
            Some(Path::new("C:\\app")),
            "C:\\a;C:\\b",
        );
        assert!(
            w.iter()
                .any(|p| p.to_string_lossy().ends_with("ffmpeg.exe"))
        );
        assert!(w.iter().any(|p| p.to_string_lossy().contains("C:\\a")));
        let m = tool_candidates(Os::MacOs, "ffmpeg", None, "");
        assert_eq!(m[0], Path::new("/opt/homebrew/bin/ffmpeg"));
    }

    #[test]
    fn duplicates_collapse() {
        let l = tool_candidates(Os::Linux, "x", None, "/a:/a:");
        assert_eq!(l.len(), 1);
    }
}
