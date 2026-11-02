//! Local filesystem backend: local drives and OS-mounted volumes (Linux, macOS, Windows).
//!
//! All I/O goes through `std::fs` on the engine's `local` job lane — the UI thread
//! never calls this directly. Errors map to typed [`FmError`]s; listings are capped
//! at [`fm_vfs::MAX_DIR_ENTRIES`] (configurable per instance for tests).

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use fm_vfs::{
    Entry, EntryKind, FmError, MAX_DIR_ENTRIES, Scheme, SpaceInfo, Vfs, VfsUri, WriteOpts,
};

/// The local-disk backend.
#[derive(Debug, Clone)]
pub struct LocalFs {
    max_entries: usize,
}

impl Default for LocalFs {
    fn default() -> Self {
        Self {
            max_entries: MAX_DIR_ENTRIES,
        }
    }
}

impl LocalFs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test/hostile-input hook: fail `read_dir` past `max_entries` instead of
    /// the production cap.
    pub fn with_max_entries(max_entries: usize) -> Self {
        Self { max_entries }
    }

    /// Only `file://` URIs are accepted; anything else is a backend error.
    fn path(&self, uri: &VfsUri) -> Result<PathBuf, FmError> {
        if uri.scheme() != Scheme::File {
            return Err(FmError::backend(
                "file",
                format!("local backend got a {} uri", uri.scheme()),
            ));
        }
        uri.local_path().ok_or(FmError::Unsupported(
            "This PC lists the drives; open a drive first",
        ))
    }
}

/// How a local uri is named in errors: the OS form (`C:\x` on Windows).
fn shown(uri: &VfsUri) -> String {
    uri.local_path_string()
        .unwrap_or_else(|| uri.path_str().to_string())
}

/// The Windows drive list ("This PC"): every letter whose root `probe` reports as a folder,
/// shown as `Label (C:)` when `label` knows the volume's name.
fn drive_entries(
    root: &VfsUri,
    probe: impl Fn(&str) -> bool,
    label: impl Fn(&str) -> Option<String>,
) -> Vec<Entry> {
    (b'A'..=b'Z')
        .filter_map(|letter| {
            let name = format!("{}:", char::from(letter));
            let drive_root = format!("{name}\\");
            if !probe(&drive_root) {
                return None;
            }
            let mut entry = Entry::new(root.join(&name).ok()?, EntryKind::Dir);
            entry.label = label(&drive_root)
                .map(|l| {
                    l.trim()
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(64)
                        .collect::<String>()
                })
                .filter(|l| !l.is_empty())
                .map(|l| format!("{l} ({name})"));
            entry.name = name;
            Some(entry)
        })
        .collect()
}

/// Volume labels by mount point (`C:\` → `Windows`), as the OS reports them.
fn volume_labels() -> std::collections::HashMap<String, String> {
    sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter_map(|d| {
            let mount = d.mount_point().to_str()?.to_ascii_uppercase();
            let name = d.name().to_str()?.to_string();
            Some((mount, name))
        })
        .collect()
}

fn map_io(e: std::io::Error, path: &str) -> FmError {
    map_platform_io(e, path, cfg!(windows))
}

fn map_platform_io(e: std::io::Error, path: &str, windows: bool) -> FmError {
    use std::io::ErrorKind as K;
    if windows && e.raw_os_error() == Some(1005) {
        return FmError::backend(
            "file",
            format!(
                "Cannot open {path}: Windows cannot recognize the drive's file system.\n\
                 The drive may be unformatted, encrypted/locked, use an unsupported file system, or be damaged.\n\
                 If encrypted, unlock or mount it with the appropriate software (for example BitLocker or VeraCrypt).\n\
                 Otherwise, check the drive in Windows Disk Management. Do not format it if it contains data you need.\n\
                 Windows error 1005."
            ),
        );
    }
    match e.kind() {
        K::NotFound => FmError::NotFound(path.to_string()),
        K::PermissionDenied => FmError::PermissionDenied(path.to_string()),
        K::AlreadyExists => FmError::AlreadyExists(path.to_string()),
        K::NotADirectory => FmError::NotADirectory(path.to_string()),
        K::IsADirectory => FmError::NotADirectory(path.to_string()),
        K::DirectoryNotEmpty => FmError::NotEmpty(path.to_string()),
        K::CrossesDevices => FmError::backend(
            "file",
            "cross-device rename — a copy+delete pass is required (change 3)",
        ),
        _ => FmError::Io(e),
    }
}

#[cfg(unix)]
fn make_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows distinguishes directory and file links; creating either needs Developer Mode or
/// the symlink privilege, which surfaces as a permission error.
#[cfg(windows)]
fn make_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    let resolved = link
        .parent()
        .map_or_else(|| PathBuf::from(target), |p| p.join(target));
    if fs::metadata(&resolved).is_ok_and(|m| m.is_dir()) {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

#[cfg(not(any(unix, windows)))]
fn make_symlink(_target: &str, _link: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "symbolic links are not supported on this platform",
    ))
}

fn kind_of(meta: &fs::Metadata, path: &Path) -> EntryKind {
    let ft = meta.file_type();
    if ft.is_symlink() {
        let target = fs::read_link(path)
            .map(|t| t.to_string_lossy().into_owned())
            .unwrap_or_default();
        EntryKind::Symlink { target }
    } else if ft.is_dir() {
        EntryKind::Dir
    } else if ft.is_file() {
        EntryKind::File
    } else {
        // fifo, socket, device, … — shown, never opened blindly.
        EntryKind::Other
    }
}

/// One listing row for `path`. `None` when the entry cannot be represented
/// (vanished between listing and stat, or its name cannot form a child URI).
fn entry_for(parent_uri: &VfsUri, path: &Path, name: String) -> Option<Entry> {
    let child_uri = parent_uri.join(&name).ok()?;
    let link_meta = fs::symlink_metadata(path).ok()?;
    let kind = kind_of(&link_meta, path);
    // Size/mtime follow the symlink so a link to a file shows the file's data.
    let follow = fs::metadata(path).ok();
    let source = follow.as_ref().unwrap_or(&link_meta);
    let size = if source.is_file() {
        Some(source.len())
    } else {
        None
    };
    let modified = source
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| UNIX_EPOCH.checked_add(d));
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        Some(source.permissions().mode() & 0o7777)
    };
    #[cfg(not(unix))]
    let permissions: Option<u32> = None;
    Some(Entry {
        uri: child_uri,
        name,
        kind,
        size,
        modified,
        permissions,
        hidden: os_hidden(&link_meta),
        link_to_dir: link_meta.file_type().is_symlink()
            && follow.as_ref().is_some_and(|m| m.is_dir()),
        label: None,
    })
}

/// The platform's own hidden flag (dot-names are handled by the engine).
fn os_hidden(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        meta.st_flags() & UF_HIDDEN != 0
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = meta;
        false
    }
}

impl Vfs for LocalFs {
    fn scheme(&self) -> Scheme {
        Scheme::File
    }

    fn read_dir(&self, uri: &VfsUri) -> Result<Vec<Entry>, FmError> {
        if uri.is_drive_list() {
            let labels = volume_labels();
            return Ok(drive_entries(
                uri,
                |root| fs::metadata(root).is_ok_and(|m| m.is_dir()),
                |root| labels.get(root).cloned(),
            ));
        }
        let path = self.path(uri)?;
        let display = &shown(uri);
        let rd = fs::read_dir(&path).map_err(|e| map_io(e, display))?;
        let mut out = Vec::new();
        for item in rd {
            let item = item.map_err(|e| map_io(e, display))?;
            let name = item.file_name().to_string_lossy().into_owned();
            let child = item.path();
            if let Some(entry) = entry_for(uri, &child, name) {
                out.push(entry);
            }
            if out.len() > self.max_entries {
                return Err(FmError::backend(
                    "file",
                    format!("directory too large (> {} entries)", self.max_entries),
                ));
            }
        }
        Ok(out)
    }

    fn stat(&self, uri: &VfsUri) -> Result<Entry, FmError> {
        if uri.is_drive_list() {
            let mut entry = Entry::new(uri.clone(), EntryKind::Dir);
            entry.name = "This PC".to_string();
            return Ok(entry);
        }
        let path = self.path(uri)?;
        let display = &shown(uri);
        let meta = fs::symlink_metadata(&path).map_err(|e| map_io(e, display))?;
        let name = uri
            .file_name()
            .map(str::to_string)
            .unwrap_or_else(|| display.to_string());
        Ok(Entry {
            uri: uri.clone(),
            name,
            kind: kind_of(&meta, &path),
            size: if meta.is_file() {
                Some(meta.len())
            } else {
                None
            },
            modified: meta.modified().ok(),
            hidden: os_hidden(&meta),
            link_to_dir: meta.file_type().is_symlink()
                && fs::metadata(&path).is_ok_and(|m| m.is_dir()),
            permissions: {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    Some(meta.permissions().mode() & 0o7777)
                }
                #[cfg(not(unix))]
                {
                    None
                }
            },
            label: None,
        })
    }

    fn open_read(&self, uri: &VfsUri) -> Result<Box<dyn Read + Send>, FmError> {
        let path = self.path(uri)?;
        let display = &shown(uri);
        let f = fs::File::open(&path).map_err(|e| map_io(e, display))?;
        Ok(Box::new(f))
    }

    fn open_read_at(&self, uri: &VfsUri, offset: u64) -> Result<Box<dyn Read + Send>, FmError> {
        use std::io::{Seek, SeekFrom};
        let path = self.path(uri)?;
        let display = &shown(uri);
        let mut f = fs::File::open(&path).map_err(|e| map_io(e, display))?;
        f.seek(SeekFrom::Start(offset))
            .map_err(|e| map_io(e, display))?;
        Ok(Box::new(f))
    }

    fn open_write(&self, uri: &VfsUri, opts: WriteOpts) -> Result<Box<dyn Write + Send>, FmError> {
        let path = self.path(uri)?;
        let display = &shown(uri);
        let mut o = fs::OpenOptions::new();
        o.write(true).create(opts.create);
        if opts.append {
            o.append(true);
        } else if opts.truncate {
            o.truncate(true);
        }
        let f = o.open(&path).map_err(|e| map_io(e, display))?;
        Ok(Box::new(f))
    }

    fn create_dir(&self, uri: &VfsUri) -> Result<(), FmError> {
        let path = self.path(uri)?;
        let display = &shown(uri);
        fs::create_dir(&path).map_err(|e| map_io(e, display))
    }

    fn create_symlink(&self, target: &str, link: &VfsUri) -> Result<(), FmError> {
        let path = self.path(link)?;
        make_symlink(target, &path).map_err(|e| map_io(e, &shown(link)))
    }

    #[cfg(unix)]
    fn set_permissions(&self, uri: &VfsUri, mode: u32) -> Result<(), FmError> {
        use std::os::unix::fs::PermissionsExt;
        let path = self.path(uri)?;
        let display = &shown(uri);
        // Symlinks have no mode of their own on Linux; refuse instead of changing the target.
        let meta = fs::symlink_metadata(&path).map_err(|e| map_io(e, display))?;
        if meta.file_type().is_symlink() {
            return Err(FmError::Unsupported(
                "permissions of a symbolic link cannot be changed",
            ));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o7777))
            .map_err(|e| map_io(e, display))
    }

    fn remove(&self, uri: &VfsUri) -> Result<(), FmError> {
        let path = self.path(uri)?;
        let display = &shown(uri);
        let meta = fs::symlink_metadata(&path).map_err(|e| map_io(e, display))?;
        if meta.is_dir() && !meta.file_type().is_symlink() {
            let mut rd = fs::read_dir(&path).map_err(|e| map_io(e, display))?;
            if rd
                .next()
                .transpose()
                .map_err(|e| map_io(e, display))?
                .is_some()
            {
                return Err(FmError::NotEmpty(display.to_string()));
            }
            fs::remove_dir(&path).map_err(|e| map_io(e, display))
        } else {
            fs::remove_file(&path).map_err(|e| map_io(e, display))
        }
    }

    fn rename(&self, from: &VfsUri, to: &VfsUri) -> Result<(), FmError> {
        let src = self.path(from)?;
        let dst = self.path(to)?;
        let display = &shown(from);
        fs::rename(&src, &dst).map_err(|e| map_io(e, display))
    }

    fn same_device(&self, a: &VfsUri, b: &VfsUri) -> bool {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            match (self.path(a), self.path(b)) {
                (Ok(pa), Ok(pb)) => match (
                    fs::symlink_metadata(&pa).map(|m| m.dev()),
                    fs::symlink_metadata(&pb).map(|m| m.dev()),
                ) {
                    (Ok(da), Ok(db)) => da == db,
                    _ => false,
                },
                _ => false,
            }
        }
        #[cfg(not(unix))]
        {
            // Conservative: never claim same-device without volume ids (v1).
            let _ = (a, b);
            false
        }
    }

    fn space(&self, uri: &VfsUri) -> Result<Option<SpaceInfo>, FmError> {
        let path = self.path(uri)?;
        let disks = sysinfo::Disks::new_with_refreshed_list();
        let mut best: Option<&sysinfo::Disk> = None;
        for d in disks.list() {
            let mp = d.mount_point();
            if path.starts_with(mp) {
                let better = match best {
                    Some(b) => mp.components().count() > b.mount_point().components().count(),
                    None => true,
                };
                if better {
                    best = Some(d);
                }
            }
        }
        Ok(best.map(|d| SpaceInfo {
            total: d.total_space(),
            available: d.available_space(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_drive_list_has_one_folder_per_existing_drive_root() {
        let root = VfsUri::root_file();
        let drives = drive_entries(
            &root,
            |p| p == "C:\\" || p == "E:\\",
            |p| (p == "C:\\").then(|| " Windows\u{7} ".to_string()),
        );
        let names: Vec<_> = drives.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["C:", "E:"]);
        let shown: Vec<_> = drives.iter().map(Entry::display_name).collect();
        assert_eq!(shown, ["Windows (C:)", "E:"]);
        assert!(drives.iter().all(|e| e.kind == EntryKind::Dir));
        let c = drives.first().expect("C:");
        assert_eq!(c.uri.path_str(), "/C:");
        assert_eq!(c.uri.parent(), Some(root.clone()));
        assert!(drive_entries(&root, |_| false, |_| None).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn this_pc_lists_drives_and_has_no_os_path() {
        let fs_ = LocalFs::new();
        let root = VfsUri::root_file();
        let drives = fs_.read_dir(&root).expect("drives");
        assert!(!drives.is_empty());
        assert_eq!(fs_.stat(&root).expect("stat").name, "This PC");
        assert!(matches!(
            fs_.create_dir(&root.join("x").expect("uri")),
            Err(_)
        ));
        let first = drives.first().expect("drive");
        fs_.read_dir(&first.uri).expect("a drive root lists");
    }

    #[test]
    fn scheme_is_file() {
        assert_eq!(LocalFs::new().scheme(), Scheme::File);
    }

    #[test]
    fn non_file_scheme_is_a_backend_error() {
        let fs_ = LocalFs::new();
        let smb = VfsUri::parse("smb://nas/share").expect("static uri");
        let err = fs_.stat(&smb).expect_err("must reject");
        assert!(matches!(err, FmError::Backend { .. }), "{err:?}");
    }

    #[test]
    fn io_mapping_covers_the_common_kinds() {
        let e = std::io::Error::from(std::io::ErrorKind::NotFound);
        assert!(matches!(map_io(e, "/x"), FmError::NotFound(_)));
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(matches!(map_io(e, "/x"), FmError::PermissionDenied(_)));
        let e = std::io::Error::from(std::io::ErrorKind::AlreadyExists);
        assert!(matches!(map_io(e, "/x"), FmError::AlreadyExists(_)));
        let e = std::io::Error::from(std::io::ErrorKind::DirectoryNotEmpty);
        assert!(matches!(map_io(e, "/x"), FmError::NotEmpty(_)));
    }

    #[test]
    fn windows_unrecognized_volume_has_safe_actionable_guidance() {
        let err = map_platform_io(std::io::Error::from_raw_os_error(1005), r"H:\", true);
        let message = err.to_string();
        for expected in [
            r"Cannot open H:\",
            "unformatted",
            "encrypted/locked",
            "unsupported file system",
            "damaged",
            "BitLocker",
            "VeraCrypt",
            "Disk Management",
            "Do not format",
            "1005",
        ] {
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn volume_guidance_does_not_replace_other_platform_errors() {
        assert!(matches!(
            map_platform_io(std::io::Error::from_raw_os_error(1005), "/x", false),
            FmError::Io(_)
        ));
        assert!(matches!(
            map_platform_io(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
                r"H:\",
                true
            ),
            FmError::PermissionDenied(_)
        ));
    }
}
