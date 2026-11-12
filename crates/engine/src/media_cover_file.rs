//! The current item's cover art as a file, for the OS media controls (MPRIS `artUrl`,
//! Windows SMTC thumbnail, macOS Now Playing artwork), which take a `file://` URL.
//!
//! The file is our own bounded decode (`media_cmds` decodes embedded art to at most
//! 512 px), re-encoded as PNG: other processes never decode the file's original,
//! untrusted image bytes. It lives in one owner-only folder per process, created with
//! an unpredictable name (`open_cmds::private_temp_dir`) and removed by
//! `sweep_open_temp` on quit or after a day. Runs on a job lane, never in a frame.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use fm_preview::ImageDoc;

/// The per-process cover folder, created on first use.
static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);

fn folder() -> Option<PathBuf> {
    let mut dir = DIR.lock().ok()?;
    if let Some(dir) = dir.as_ref().filter(|dir| dir.is_dir()) {
        return Some(dir.clone());
    }
    let made = crate::open_cmds::private_temp_dir().ok()?;
    *dir = Some(made.clone());
    Some(made)
}

/// Write `image` (first frame) as a fresh PNG and remove earlier covers. `None` when the
/// image is malformed or the file cannot be written: the controls then show no art.
pub(crate) fn export(image: &ImageDoc) -> Option<PathBuf> {
    export_in(&folder()?, image)
}

fn export_in(dir: &Path, image: &ImageDoc) -> Option<PathBuf> {
    let (rgba, _) = image.frames.first()?;
    let expected = u64::from(image.width) * u64::from(image.height) * 4;
    if image.width == 0 || image.height == 0 || rgba.len() as u64 != expected {
        return None;
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            rgba,
            image.width,
            image.height,
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("cover-"))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let path = dir.join(format!("cover-{}.png", SEQ.fetch_add(1, Ordering::Relaxed)));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&path).ok()?;
    file.write_all(&png).ok()?;
    Some(path)
}

use image::ImageEncoder as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(width: u32, height: u32, bytes: usize) -> ImageDoc {
        ImageDoc {
            width,
            height,
            frames: vec![(vec![200u8; bytes], 0)],
            original: (width, height),
            format: "PNG",
        }
    }

    #[test]
    fn a_cover_is_written_as_an_owner_only_png_and_replaces_the_last() {
        let dir = tempfile::tempdir().expect("dir");
        let first = export_in(dir.path(), &doc(4, 2, 32)).expect("first");
        let bytes = std::fs::read(&first).expect("read");
        assert_eq!(bytes.get(..8), Some(b"\x89PNG\r\n\x1a\n".as_slice()));
        let decoded = image::load_from_memory(&bytes).expect("valid png");
        assert_eq!((decoded.width(), decoded.height()), (4, 2));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first)
                .expect("meta")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let second = export_in(dir.path(), &doc(1, 1, 4)).expect("second");
        assert_ne!(first, second);
        assert!(!first.exists(), "the previous cover is removed");
        assert!(second.exists());
    }

    #[test]
    fn malformed_images_write_nothing() {
        let dir = tempfile::tempdir().expect("dir");
        assert!(
            export_in(dir.path(), &doc(4, 2, 31)).is_none(),
            "short pixels"
        );
        assert!(export_in(dir.path(), &doc(0, 2, 0)).is_none(), "zero width");
        let mut empty = doc(1, 1, 4);
        empty.frames.clear();
        assert!(export_in(dir.path(), &empty).is_none(), "no frame");
        assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
    }
}
