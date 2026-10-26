//! [`VfsUri`] — the one path type for every location the product can touch.
//!
//! Parsing is **total over hostile input**: control characters, overlong
//! components, relative paths and embedded credentials are rejected with
//! [`FmError::InvalidUri`]; `.`/`..` are normalized (clamped at the root) so
//! a normalized path can never escape its parent. Display round-trips.

use std::fmt;

use crate::error::FmError;
use crate::{MAX_COMPONENT_BYTES, MAX_PATH_BYTES};

/// Which backend serves a URI.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Scheme {
    /// Local filesystem (`file:///…`). Also used for Windows drive and UNC paths.
    File,
    /// SMB share browsed by the in-app client (`smb://host[:port]/share/…`).
    Smb,
    /// SSH/SFTP server (`sftp://[user@]host[:port]/…`).
    Sftp,
    /// The virtual "Network" folder (`network:///`) listing discovered hosts.
    Network,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Smb => "smb",
            Self::Sftp => "sftp",
            Self::Network => "network",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "file" => Some(Self::File),
            "smb" => Some(Self::Smb),
            "sftp" => Some(Self::Sftp),
            "network" => Some(Self::Network),
            _ => None,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `user@host:port` part of a remote URI. Empty `host` means "local" (file scheme).
#[derive(
    Clone,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Authority {
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
}

impl Authority {
    pub fn local() -> Self {
        Self {
            user: None,
            host: String::new(),
            port: None,
        }
    }

    pub fn is_local(&self) -> bool {
        self.host.is_empty()
    }
}

impl fmt::Display for Authority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_local() {
            return Ok(());
        }
        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }
        write!(f, "{}", self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

/// An absolute, normalized location: `scheme://authority/path`.
///
/// The `path` is always absolute (`/`-separated, no trailing slash except the
/// root, no `.`/`..` components). For SMB the first path segment is the share.
#[derive(
    Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct VfsUri {
    scheme: Scheme,
    authority: Authority,
    /// Absolute, normalized, `/`-separated; always starts with `/`.
    path: String,
}

impl VfsUri {
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn authority(&self) -> &Authority {
        &self.authority
    }

    /// The path component (absolute, `/`-separated). For SMB this includes the share.
    pub fn path_str(&self) -> &str {
        &self.path
    }

    /// SMB share = first path segment, if any (`None` for non-SMB or server root).
    pub fn share(&self) -> Option<&str> {
        if self.scheme != Scheme::Smb {
            return None;
        }
        self.path
            .strip_prefix('/')
            .and_then(|rest| rest.split('/').next())
            .filter(|s| !s.is_empty())
    }

    /// Parse any accepted URI or raw path form. Never panics; hostile input is an error.
    pub fn parse(input: &str) -> Result<Self, FmError> {
        // Cheap total guards before any allocation.
        if input.len() > MAX_PATH_BYTES {
            return Err(FmError::invalid_uri("input too long"));
        }
        if input.chars().any(|c| c.is_control()) {
            return Err(FmError::invalid_uri("control character in uri"));
        }
        if input.is_empty() {
            return Err(FmError::invalid_uri("empty uri"));
        }

        if let Some(scheme_end) = input.find("://") {
            let scheme_str = &input[..scheme_end];
            let scheme = Scheme::parse(scheme_str)
                .ok_or_else(|| FmError::invalid_uri(format!("unknown scheme {scheme_str}")))?;
            let rest = &input[scheme_end + 3..];
            Self::parse_with_scheme(scheme, rest, input)
        } else {
            Self::parse_raw_path(input)
        }
    }

    /// Convenience: parse raw bytes (non-UTF-8 is rejected, never lossily converted).
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, FmError> {
        let s = std::str::from_utf8(bytes).map_err(|_| FmError::invalid_uri("non-utf-8 uri"))?;
        Self::parse(s)
    }

    fn parse_with_scheme(scheme: Scheme, rest: &str, original: &str) -> Result<Self, FmError> {
        // Split authority from path at the first '/'.
        let (auth_part, path_part) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, ""),
        };

        let authority = match scheme {
            Scheme::File => {
                // file:// must have an empty authority or "localhost".
                if auth_part.is_empty() || auth_part == "localhost" {
                    Authority::local()
                } else {
                    return Err(FmError::invalid_uri(original));
                }
            }
            Scheme::Smb | Scheme::Sftp => parse_authority(auth_part, scheme, original)?,
            // `network:///` is the only location of this scheme.
            Scheme::Network => {
                if auth_part.is_empty() && matches!(path_part, "" | "/") {
                    return Ok(Self::network());
                }
                return Err(FmError::invalid_uri(original));
            }
        };

        let path = normalize_path(if path_part.is_empty() { "/" } else { path_part })?;
        Ok(Self {
            scheme,
            authority,
            path,
        })
    }

    /// Raw path forms: `/abs`, `C:\…` / `C:/…`, UNC `\\server\share\…`.
    fn parse_raw_path(input: &str) -> Result<Self, FmError> {
        if input.starts_with('/') {
            // POSIX absolute, also `//server/share` (UNC with forward slashes).
            let path = normalize_path(input)?;
            return Ok(Self {
                scheme: Scheme::File,
                authority: Authority::local(),
                path,
            });
        }
        // Windows drive: `C:\x` or `C:/x` (a bare `C:` or `C:no-slash` is drive-relative
        // and is rejected — we only accept fully-qualified paths).
        let bytes = input.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\')
        {
            let swapped = input.replace('\\', "/");
            let path = normalize_path(&format!("/{swapped}"))?;
            return Ok(Self {
                scheme: Scheme::File,
                authority: Authority::local(),
                path,
            });
        }
        // UNC: `\\server\share\...` → file scheme, path `//server/share/...` (std::fs
        // on Windows accepts this form natively; no in-app SMB session required).
        if input.starts_with("\\\\") {
            let swapped = input.replace('\\', "/");
            // `//server/share/...` — keep the two leading slashes collapsed to one
            // so normalization yields `/server/share/...`? No: preserve server as
            // first segment: strip the leading `//` then normalize as `/server/...`.
            let trimmed = swapped.trim_start_matches('/');
            let path = normalize_path(&format!("/{trimmed}"))?;
            return Ok(Self {
                scheme: Scheme::File,
                authority: Authority::local(),
                path,
            });
        }
        Err(FmError::invalid_uri(input))
    }

    /// The local filesystem root (`file:///`), available without parsing.
    /// The virtual Network folder.
    pub fn network() -> Self {
        Self {
            scheme: Scheme::Network,
            authority: Authority::local(),
            path: "/".to_string(),
        }
    }

    pub fn root_file() -> Self {
        Self {
            scheme: Scheme::File,
            authority: Authority::local(),
            path: "/".to_string(),
        }
    }

    /// Root of this URI (same scheme/authority, path `/`).
    pub fn root(&self) -> Self {
        Self {
            scheme: self.scheme,
            authority: self.authority.clone(),
            path: "/".to_string(),
        }
    }

    /// Parent URI, or `None` at the root.
    pub fn parent(&self) -> Option<Self> {
        if self.path == "/" {
            return None;
        }
        let trimmed = self.path.rsplit_once('/').map_or("/", |(head, _)| head);
        Some(Self {
            scheme: self.scheme,
            authority: self.authority.clone(),
            path: if trimmed.is_empty() {
                "/".into()
            } else {
                trimmed.to_string()
            },
        })
    }

    /// Last path segment (`None` at the root).
    pub fn file_name(&self) -> Option<&str> {
        if self.path == "/" {
            return None;
        }
        self.path.rsplit('/').next().filter(|s| !s.is_empty())
    }

    /// Append a single path segment. Rejects empty names, `/`, `..`, control chars.
    pub fn join(&self, name: &str) -> Result<Self, FmError> {
        if name.is_empty()
            || name.contains('/')
            || name == "."
            || name == ".."
            || name.chars().any(|c| c.is_control())
            || name.len() > MAX_COMPONENT_BYTES
        {
            return Err(FmError::invalid_uri(name));
        }
        let mut path = String::with_capacity(self.path.len() + 1 + name.len());
        path.push_str(&self.path);
        if self.path != "/" {
            path.push('/');
        }
        path.push_str(name);
        if path.len() > MAX_PATH_BYTES {
            return Err(FmError::invalid_uri("path too long after join"));
        }
        Ok(Self {
            scheme: self.scheme,
            authority: self.authority.clone(),
            path,
        })
    }

    /// Strict ancestor: `self` is on the path from `other` to the root, `self != other`.
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        self != other && self.is_same_or_descendant_of(other)
    }

    /// Same location, or an ancestor of it (scheme + authority must match).
    pub fn is_same_or_descendant_of(&self, other: &Self) -> bool {
        if self.scheme != other.scheme || self.authority != other.authority {
            return false;
        }
        if self.path == "/" {
            return true;
        }
        other.path == self.path || other.path.starts_with(&format!("{}/", self.path))
    }

    /// Canonical string form; `parse` accepts it back.
    pub fn as_str(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for VfsUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme {
            Scheme::File => write!(f, "file://{}", self.path),
            Scheme::Smb => write!(f, "smb://{}{}", self.authority, self.path),
            Scheme::Sftp => write!(f, "sftp://{}{}", self.authority, self.path),
            Scheme::Network => f.write_str("network:///"),
        }
    }
}

impl std::str::FromStr for VfsUri {
    type Err = FmError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

fn parse_authority(part: &str, scheme: Scheme, original: &str) -> Result<Authority, FmError> {
    if part.is_empty() {
        return Err(FmError::invalid_uri(original));
    }
    // Reject embedded passwords (`user:pass@host`) — credentials never live in a URI.
    let (userinfo, hostport) = match part.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, part),
    };
    let (user, password_present) = match userinfo {
        Some(u) => match u.split_once(':') {
            // `user:pass@` — reject.
            Some(_) => return Err(FmError::invalid_uri("credentials in uri")),
            None => (Some(u.to_string()), false),
        },
        None => (None, false),
    };
    let _ = password_present;

    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p
                .parse()
                .map_err(|_| FmError::invalid_uri(format!("bad port in {original}")))?;
            (h.to_string(), Some(port))
        }
        None => (hostport.to_string(), None),
    };
    if host.is_empty() {
        return Err(FmError::invalid_uri(original));
    }
    // scheme only used to reject user@host for smb? Allow user for both; user is not secret.
    let _ = scheme;
    Ok(Authority { user, host, port })
}

/// Normalize an absolute `/`-separated path: drop empty and `.` segments, clamp `..`
/// at the root, enforce component and total length caps.
fn normalize_path(raw: &str) -> Result<String, FmError> {
    if raw.len() > MAX_PATH_BYTES {
        return Err(FmError::invalid_uri("path too long"));
    }
    let mut out: Vec<&str> = Vec::new();
    let mut len = 1usize; // leading '/'
    for seg in raw.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                out.pop();
            }
            s => {
                if s.len() > MAX_COMPONENT_BYTES {
                    return Err(FmError::invalid_uri("path component too long"));
                }
                len += 1 + s.len();
                if len > MAX_PATH_BYTES {
                    return Err(FmError::invalid_uri("path too long"));
                }
                out.push(s);
            }
        }
    }
    if out.is_empty() {
        Ok("/".to_string())
    } else {
        let mut s = String::with_capacity(len);
        for seg in out {
            s.push('/');
            s.push_str(seg);
        }
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_posix_absolute() {
        let uri = VfsUri::parse("/home/alice/Docs").unwrap();
        assert_eq!(uri.scheme(), Scheme::File);
        assert!(uri.authority().is_local());
        assert_eq!(uri.path_str(), "/home/alice/Docs");
        assert_eq!(uri.file_name(), Some("Docs"));
        assert_eq!(uri.to_string(), "file:///home/alice/Docs");
    }

    #[test]
    fn parses_file_url() {
        let uri = VfsUri::parse("file:///var/log").unwrap();
        assert_eq!(uri.path_str(), "/var/log");
        assert_eq!(VfsUri::parse("file://localhost/var/log").unwrap(), uri);
    }

    #[test]
    fn rejects_file_url_with_host() {
        assert!(matches!(
            VfsUri::parse("file://evil/var/log"),
            Err(FmError::InvalidUri(_))
        ));
    }

    #[test]
    fn parses_windows_drive() {
        let uri = VfsUri::parse(r"C:\Users\alice").unwrap();
        assert_eq!(uri.scheme(), Scheme::File);
        assert_eq!(uri.path_str(), "/C:/Users/alice");
        let uri2 = VfsUri::parse("C:/Users/alice").unwrap();
        assert_eq!(uri, uri2);
    }

    #[test]
    fn parses_unc_as_file() {
        let uri = VfsUri::parse(r"\\nas\share\x").unwrap();
        assert_eq!(uri.scheme(), Scheme::File);
        assert_eq!(uri.path_str(), "/nas/share/x");
    }

    #[test]
    fn network_uri_round_trips_and_rejects_everything_else() {
        let uri = VfsUri::parse("network:///").unwrap();
        assert_eq!(uri, VfsUri::network());
        assert_eq!(VfsUri::parse(&uri.to_string()).unwrap(), uri);
        assert_eq!(VfsUri::parse("network://").unwrap(), uri);
        assert!(uri.parent().is_none());
        for bad in [
            "network://host/",
            "network:///x",
            "network:///../x",
            "network://a@b/",
        ] {
            assert!(VfsUri::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_smb_with_share() {
        let uri = VfsUri::parse("smb://nas:445/media/movies").unwrap();
        assert_eq!(uri.scheme(), Scheme::Smb);
        assert_eq!(uri.authority().host, "nas");
        assert_eq!(uri.authority().port, Some(445));
        assert_eq!(uri.share(), Some("media"));
        assert_eq!(uri.path_str(), "/media/movies");
        assert_eq!(uri.to_string(), "smb://nas:445/media/movies");
    }

    #[test]
    fn parses_sftp() {
        let uri = VfsUri::parse("sftp://alice@example.com:2222/srv/www").unwrap();
        assert_eq!(uri.scheme(), Scheme::Sftp);
        assert_eq!(uri.authority().user.as_deref(), Some("alice"));
        assert_eq!(uri.authority().port, Some(2222));
        assert_eq!(uri.path_str(), "/srv/www");
    }

    #[test]
    fn rejects_embedded_password() {
        assert!(matches!(
            VfsUri::parse("sftp://alice:hunter2@example.com/x"),
            Err(FmError::InvalidUri(_))
        ));
    }

    #[test]
    fn rejects_control_chars_and_nul() {
        assert!(VfsUri::parse("/tmp/a\0b").is_err());
        assert!(VfsUri::parse("/tmp/a\nb").is_err());
        assert!(VfsUri::parse("/tmp/\u{7}").is_err());
    }

    #[test]
    fn rejects_relative() {
        assert!(VfsUri::parse("relative/path").is_err());
        assert!(VfsUri::parse("").is_err());
        assert!(VfsUri::parse("C:no-slash").is_err());
    }

    #[test]
    fn rejects_overlong_component() {
        let long = "a".repeat(MAX_COMPONENT_BYTES + 1);
        let input = format!("/{long}");
        assert!(matches!(VfsUri::parse(&input), Err(FmError::InvalidUri(_))));
    }

    #[test]
    fn rejects_overlong_path() {
        // Many medium components: total > 8 KiB → Err (never a 10 000-char allocation).
        let seg = "b".repeat(1000);
        let mut input = String::new();
        for _ in 0..20 {
            input.push('/');
            input.push_str(&seg);
        }
        assert!(input.len() > 10_000);
        assert!(VfsUri::parse(&input).is_err());
    }

    #[test]
    fn normalizes_dot_dot_and_dots() {
        let uri = VfsUri::parse("/a/b/../c/./d/../../..").unwrap();
        assert_eq!(uri.path_str(), "/");
        let uri = VfsUri::parse("/a/../../etc/passwd").unwrap();
        assert_eq!(uri.path_str(), "/etc/passwd"); // clamped, never escapes
    }

    #[test]
    fn parent_and_join() {
        let uri = VfsUri::parse("/home/alice").unwrap();
        let parent = uri.parent().unwrap();
        assert_eq!(parent.path_str(), "/home");
        assert_eq!(parent.parent().unwrap().path_str(), "/");
        assert_eq!(parent.parent().unwrap().parent(), None);

        let joined = uri.join("Docs").unwrap();
        assert_eq!(joined.path_str(), "/home/alice/Docs");
        assert_eq!(joined.parent(), Some(uri.clone()));

        for bad in ["", "/", "..", ".", "a/b", "x\0"] {
            assert!(uri.join(bad).is_err(), "join({bad:?}) should fail");
        }
    }

    #[test]
    fn ancestor_checks() {
        let root = VfsUri::parse("file:///").unwrap();
        let home = VfsUri::parse("/home/alice").unwrap();
        let docs = VfsUri::parse("/home/alice/Docs").unwrap();
        assert!(root.is_ancestor_of(&home));
        assert!(home.is_ancestor_of(&docs));
        assert!(root.is_ancestor_of(&docs));
        assert!(!docs.is_ancestor_of(&home));
        assert!(!home.is_ancestor_of(&home));
        assert!(home.is_same_or_descendant_of(&home));
        let other = VfsUri::parse("sftp://alice@example.com/home/alice").unwrap();
        assert!(!root.is_ancestor_of(&other));
    }

    #[test]
    fn display_round_trips() {
        for input in [
            "file:///home/alice",
            "file:///C:/Users/x",
            "smb://nas:445/media/movies",
            "sftp://alice@example.com:2222/srv/www",
        ] {
            let uri = VfsUri::parse(input).unwrap();
            let again = VfsUri::parse(&uri.to_string()).unwrap();
            assert_eq!(uri, again, "round trip of {input}");
        }
    }

    #[test]
    fn parse_bytes_rejects_non_utf8() {
        assert!(VfsUri::parse_bytes(&[0xff, 0xfe]).is_err());
        assert!(VfsUri::parse_bytes(b"/tmp/ok").is_ok());
    }

    #[test]
    fn smb_share_helpers() {
        let root = VfsUri::parse("smb://nas/").unwrap();
        assert_eq!(root.share(), None);
        let share_only = VfsUri::parse("smb://nas/media").unwrap();
        assert_eq!(share_only.share(), Some("media"));
    }
}
