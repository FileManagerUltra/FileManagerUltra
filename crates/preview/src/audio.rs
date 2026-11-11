//! Audio metadata is loaded on the preview job lane, without opening an output device.

use std::path::PathBuf;

use fm_media::AudioDecoder;
use fm_vfs::{Entry, FmError};

use crate::{Limits, MetaDoc, Preview, base_meta, image_prev};

#[derive(Clone, Debug, PartialEq)]
pub struct AudioDoc {
    pub path: PathBuf,
    pub duration: Option<f64>,
}

pub fn render(entry: &Entry, limits: &Limits) -> Result<Preview, FmError> {
    let mut meta = MetaDoc {
        rows: base_meta(entry),
        hint: None,
        poster: None,
        video: None,
        audio: None,
    };
    let Some(path) = entry.uri.local_path() else {
        meta.hint = Some("Audio playback needs a local file; copy it locally to play.".into());
        return Ok(Preview::Meta(meta));
    };
    fm_media::set_tool_locator(crate::video::find_tool);
    let decoder = AudioDecoder::open(&path)?;
    let info = decoder.info();
    for (name, value) in [
        ("Title", &info.tags.title),
        ("Artist", &info.tags.artist),
        ("Album", &info.tags.album),
        ("Genre", &info.tags.genre),
    ] {
        if let Some(value) = value {
            meta.rows.push((name.into(), value.clone()));
        }
    }
    meta.rows.push((
        "Audio".into(),
        format!(
            "{} Hz, {} channels",
            info.format.rate(),
            info.format.channels(),
        ),
    ));
    if let Some(cover) = &info.tags.cover {
        match image_prev::decode_still(entry, &cover.bytes, limits) {
            Preview::Image(image) => meta.poster = Some(Box::new(image)),
            _ => meta.hint = Some("Embedded cover art could not be decoded safely.".into()),
        }
    }
    meta.audio = Some(AudioDoc {
        path,
        duration: info.duration,
    });
    Ok(Preview::Meta(meta))
}
