//! `image.copy`: put the full-resolution pixels of an image file on the OS clipboard.
//!
//! The file is read and decoded on the backend's job lane (never the UI thread); the result
//! lands in [`Session::clipboard_image`], which the app hands to the system clipboard.

use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fm_preview::thumbs::{ThumbKind, classify};
use fm_vfs::{FmError, VfsUri};

use crate::backends::lane_for;
use crate::file_cmds::{focused_selection, object, parse_uri};
use crate::registry::{CommandSpec, Params};
use crate::session::Session;

/// Source files above this size are refused (matches the preview limit).
const MAX_BYTES: u64 = 64 << 20;
/// 50 megapixels ≈ 200 MB of RGBA.
const MAX_PIXELS: u64 = 50_000_000;

/// Decoded RGBA pixels waiting for the app to put them on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct ImageClip {
    inbox: Arc<Mutex<Vec<Result<ClipImage, String>>>>,
    pending: Arc<AtomicUsize>,
}

impl ImageClip {
    pub fn is_busy(&self) -> bool {
        self.pending.load(Ordering::Relaxed) > 0
    }
}

fn is_image(uri: &VfsUri) -> bool {
    uri.file_name()
        .is_some_and(|n| classify(n) == ThumbKind::Image)
}

fn load(uri: &VfsUri, vfs: &dyn fm_vfs::Vfs) -> Result<ClipImage, String> {
    let mut bytes = Vec::new();
    vfs.open_read(uri)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("the image file is too large to copy".into());
    }
    let (w, h, rgba) = fm_preview::image_prev::decode_original(&bytes, MAX_PIXELS)?;
    Ok(ClipImage {
        width: w as usize,
        height: h as usize,
        rgba,
    })
}

fn target(s: &Session, params: &Params) -> Result<VfsUri, FmError> {
    let uri = match object(params)?.and_then(|m| m.get("uri")) {
        Some(v) => parse_uri(v)?,
        None => match focused_selection(s).as_slice() {
            [one] => one.clone(),
            _ => return Err(FmError::InvalidParams("select one image to copy".into())),
        },
    };
    if !is_image(&uri) {
        return Err(FmError::Unsupported("only image files can be copied"));
    }
    Ok(uri)
}

fn selected_image(s: &Session) -> bool {
    matches!(focused_selection(s).as_slice(), [one] if is_image(one))
}

impl Session {
    /// Collect finished image decodes; returns whether anything changed.
    pub(crate) fn pump_image_clip(&mut self) -> bool {
        let done: Vec<_> = match self.image_clip.inbox.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(p) => std::mem::take(&mut *p.into_inner()),
        };
        let changed = !done.is_empty();
        for result in done {
            match result {
                Ok(img) => {
                    self.clipboard_image = Some(img);
                    self.info = Some((
                        "Copied image to the clipboard".to_string(),
                        std::time::Instant::now(),
                    ));
                }
                Err(why) => self.notice = Some(format!("Could not copy the image: {why}")),
            }
        }
        changed
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "image.copy",
        label: "Copy Image",
        menu: &["Edit", "Copy Image"],
        shortcut: None,
        params: "{uri?: URI of an image file (default: the single selected image)}",
        enabled: selected_image,
        run: |s, params: &Params| {
            let uri = target(s, params)?;
            let vfs = s.backends.require(&uri)?;
            let (inbox, pending) = (s.image_clip.inbox.clone(), s.image_clip.pending.clone());
            pending.fetch_add(1, Ordering::Relaxed);
            s.jobs.submit(lane_for(uri.scheme()), move |_ctx| {
                let result = load(&uri, &*vfs);
                if let Ok(mut g) = inbox.lock() {
                    g.push(result);
                }
                pending.fetch_sub(1, Ordering::Relaxed);
                Ok(())
            });
            Ok(())
        },
    }]
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use fm_testkit::MemFs;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut out = Vec::new();
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("png");
        out
    }

    fn session(name: &str, bytes: &[u8]) -> Session {
        let mut s = Session::new().expect("session");
        let mem = MemFs::new();
        mem.mkdirs_str("/d").expect("dir");
        mem.put_str(&format!("/d/{name}"), bytes).expect("file");
        s.backends.register(Arc::new(mem));
        s
    }

    fn wait(s: &mut Session) {
        let end = Instant::now() + Duration::from_secs(5);
        while s.image_clip.is_busy() || s.clipboard_image.is_none() && s.notice.is_none() {
            assert!(Instant::now() < end, "timed out");
            s.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        s.poll();
    }

    #[test]
    fn copies_original_size_pixels() {
        // Larger than the preview edge: the clipboard must not get the downscaled copy.
        let mut s = session("big.png", &png(2600, 3));
        s.dispatch("image.copy", &json!({"uri": "/d/big.png"}))
            .expect("dispatch");
        wait(&mut s);
        let img = s.clipboard_image.take().expect("image");
        assert_eq!((img.width, img.height), (2600, 3));
        assert_eq!(img.rgba.len(), 2600 * 3 * 4);
        assert_eq!(img.rgba.get(..4), Some(&[10, 20, 30, 255][..]));
    }

    #[test]
    fn rejects_non_images_and_bad_params() {
        let mut s = session("a.txt", b"hi");
        assert!(
            s.dispatch("image.copy", &json!({"uri": "/d/a.txt"}))
                .is_err()
        );
        assert!(s.dispatch("image.copy", &json!({"uri": 5})).is_err());
        assert!(s.dispatch("image.copy", &json!({})).is_err());
    }

    #[test]
    fn corrupt_image_reports_a_notice() {
        let mut s = session("bad.png", b"not a png");
        s.dispatch("image.copy", &json!({"uri": "/d/bad.png"}))
            .expect("dispatch");
        wait(&mut s);
        assert!(s.clipboard_image.is_none());
        assert!(s.notice.is_some_and(|n| n.contains("Could not copy")));
    }
}
