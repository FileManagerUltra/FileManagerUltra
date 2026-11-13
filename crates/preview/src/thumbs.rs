//! Directory thumbnails: which files get one, how it is made, and the on-disk PNG cache.
//! Only local files are thumbnailed (a remote folder would pull every file over the wire).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fm_vfs::{Entry, Scheme, Vfs};

use crate::{Limits, Preview, image_prev, text, video};

/// Longest edges of the Small / Medium / Large presets (px).
pub const EDGES: [u32; 3] = [160, 256, 512];
/// Cached files larger than this are ignored (a thumbnail is never this big).
const MAX_CACHED_BYTES: u64 = 8 << 20;
const CACHE_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbKind {
    Image,
    Video,
    /// SVG or SVGZ, drawn by the safe SVG renderer.
    Svg,
    Other,
}

/// Decides from the file name alone, so it costs no I/O.
pub fn classify(name: &str) -> ThumbKind {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => ThumbKind::Image,
        "svg" | "svgz" => ThumbKind::Svg,
        "mp4" | "m4v" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "flv" | "mpg" | "mpeg" | "3gp"
        | "ogv" | "vob" | "m2ts" | "mts" => ThumbKind::Video,
        _ => ThumbKind::Other,
    }
}

/// RGBA pixels, longest edge at most the requested edge.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumb {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Thumb {
    pub fn bytes(&self) -> usize {
        self.rgba.len()
    }

    pub fn valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.rgba.len() == (self.width as usize) * (self.height as usize) * 4
    }
}

fn is_local(entry: &Entry) -> bool {
    entry.uri.scheme() == Scheme::File && entry.uri.authority().is_local()
}

/// Build a thumbnail; `None` when the file is not a (decodable) image or video.
pub fn make(vfs: &dyn Vfs, entry: &Entry, edge: u32) -> Option<Thumb> {
    make_as(vfs, entry, edge, classify(&entry.name))
}

/// Build a thumbnail with the configured media association; decoding remains bounded.
pub fn make_as(vfs: &dyn Vfs, entry: &Entry, edge: u32, kind: ThumbKind) -> Option<Thumb> {
    if !is_local(entry) || !entry.is_file() && !entry.is_symlink() {
        return None;
    }
    let limits = Limits {
        image_edge: edge,
        ..Limits::default()
    };
    let doc = match kind {
        ThumbKind::Image => {
            if entry.size.is_some_and(|s| s > limits.image_bytes) {
                return None;
            }
            let (bytes, more) = text::read_capped(vfs, entry, limits.image_bytes as usize).ok()?;
            if more {
                return None;
            }
            match image_prev::decode_still(entry, &bytes, &limits) {
                Preview::Image(doc) => doc,
                _ => return None,
            }
        }
        ThumbKind::Video => video::thumbnail(entry, edge)?,
        ThumbKind::Svg => return crate::svg::thumbnail(vfs, entry, edge),
        ThumbKind::Other => return None,
    };
    let (rgba, _) = doc.frames.into_iter().next()?;
    let thumb = Thumb {
        width: doc.width,
        height: doc.height,
        rgba,
    };
    thumb.valid().then_some(thumb)
}

/// PNG files in an owner-only directory, named by a hash of path, mtime, size and edge, so a
/// changed file never shows a stale picture.
#[derive(Clone, Debug)]
pub struct DiskCache {
    dir: PathBuf,
}

impl DiskCache {
    pub fn open(dir: PathBuf) -> Option<Self> {
        std::fs::create_dir_all(&dir).ok()?;
        owner_only_dir(&dir);
        Some(Self { dir })
    }

    pub fn key(entry: &Entry, edge: u32) -> String {
        let nanos = entry
            .modified
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let digest = |salt: u64| {
            let mut h = DefaultHasher::new();
            (
                salt,
                CACHE_VERSION,
                entry.uri.path_str(),
                nanos,
                entry.size,
                edge,
            )
                .hash(&mut h);
            h.finish()
        };
        format!("{:016x}{:016x}", digest(1), digest(2))
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.png"))
    }

    pub fn get(&self, key: &str) -> Option<Thumb> {
        let path = self.path(key);
        if std::fs::metadata(&path).ok()?.len() > MAX_CACHED_BYTES {
            return None;
        }
        let bytes = std::fs::read(&path).ok()?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?;
        let rgba = img.to_rgba8();
        let thumb = Thumb {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
        };
        thumb.valid().then_some(thumb)
    }

    pub fn put(&self, key: &str, thumb: &Thumb) {
        let mut png = Vec::new();
        let enc = image::codecs::png::PngEncoder::new_with_quality(
            Cursor::new(&mut png),
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        );
        let ok = image::ImageEncoder::write_image(
            enc,
            &thumb.rgba,
            thumb.width,
            thumb.height,
            image::ExtendedColorType::Rgba8,
        )
        .is_ok();
        if !ok {
            return;
        }
        let tmp = self.dir.join(format!("{key}.tmp{}", std::process::id()));
        if write_owner_only(&tmp, &png).is_ok() && std::fs::rename(&tmp, self.path(key)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Delete every cached thumbnail; returns how many files went.
    pub fn clear(&self) -> usize {
        let mut n = 0;
        for e in self.files() {
            n += usize::from(std::fs::remove_file(e.0).is_ok());
        }
        n
    }

    /// Drop the oldest files until the cache is at most `max_bytes`.
    pub fn prune(&self, max_bytes: u64) {
        let mut files = self.files();
        let mut total: u64 = files.iter().map(|f| f.2).sum();
        if total <= max_bytes {
            return;
        }
        files.sort_by_key(|f| f.1);
        for (path, _, len) in files {
            if total <= max_bytes {
                break;
            }
            if std::fs::remove_file(path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }

    fn files(&self) -> Vec<(PathBuf, SystemTime, u64)> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.filter_map(Result::ok)
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x == "png" || x == "tmp")
            })
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                m.is_file()
                    .then(|| (e.path(), m.modified().unwrap_or(UNIX_EPOCH), m.len()))
            })
            .collect()
    }
}

#[cfg(unix)]
fn owner_only_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn owner_only_dir(_dir: &Path) {}

fn write_owner_only(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_vfs::{EntryKind, VfsUri};

    fn entry(path: &str, size: u64) -> Entry {
        let mut e = Entry::new(VfsUri::parse(path).expect("uri"), EntryKind::File);
        e.size = Some(size);
        e
    }

    #[test]
    fn classifies_by_extension() {
        assert_eq!(classify("a.JPG"), ThumbKind::Image);
        assert_eq!(classify("clip.mkv"), ThumbKind::Video);
        assert_eq!(classify("logo.SVG"), ThumbKind::Svg);
        assert_eq!(classify("logo.svgz"), ThumbKind::Svg);
        assert_eq!(classify("notes.txt"), ThumbKind::Other);
        assert_eq!(classify("noext"), ThumbKind::Other);
    }

    #[test]
    fn cache_key_tracks_size_and_edge() {
        let a = DiskCache::key(&entry("/x/a.png", 10), 160);
        assert_eq!(a, DiskCache::key(&entry("/x/a.png", 10), 160));
        assert_ne!(a, DiskCache::key(&entry("/x/a.png", 11), 160));
        assert_ne!(a, DiskCache::key(&entry("/x/a.png", 10), 256));
        assert_ne!(a, DiskCache::key(&entry("/x/b.png", 10), 160));
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn disk_cache_round_trips_prunes_and_clears() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = DiskCache::open(dir.path().join("t")).expect("open");
        let thumb = Thumb {
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 128],
        };
        cache.put("k1", &thumb);
        assert_eq!(cache.get("k1"), Some(thumb.clone()));
        assert_eq!(cache.get("missing"), None);
        cache.put("k2", &thumb);
        cache.prune(0);
        assert_eq!(cache.get("k1"), None);
        cache.put("k3", &thumb);
        assert_eq!(cache.clear(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn cache_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tmp");
        let root = dir.path().join("t");
        let cache = DiskCache::open(root.clone()).expect("open");
        cache.put(
            "k",
            &Thumb {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            },
        );
        let mode = |p: &Path| std::fs::metadata(p).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(mode(&root).ok(), Some(0o700));
        assert_eq!(mode(&root.join("k.png")).ok(), Some(0o600));
    }

    #[test]
    fn corrupt_cache_file_is_a_miss() {
        let dir = tempfile::tempdir().expect("tmp");
        let cache = DiskCache::open(dir.path().to_path_buf()).expect("open");
        std::fs::write(dir.path().join("bad.png"), b"not a png").expect("write");
        assert_eq!(cache.get("bad"), None);
    }
}
