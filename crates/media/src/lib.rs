//! Shared media playback primitives. Device discovery, decoding and resampling belong on
//! workers; the UI reads snapshots and never touches an audio device.

mod buffer;
mod decoder;
mod device;
mod eq;
mod pipeline;
mod player;
mod playlist;
mod queue;

pub use buffer::{AudioConsumer, AudioProducer, Clock, audio_buffer};
pub use decoder::{AudioDecoder, AudioInfo, AudioTags, CoverArt, DecodedAudio};
pub use device::{AudioSink, DeviceSink};
pub use eq::{BAND_HZ, BANDS, EqSettings, MAX_BAND_DB, PRESETS};
pub use pipeline::{AudioFormat, Gain, Pipeline};
pub use player::{AudioPlayer, PlaybackState, PlaybackStatus};
pub use playlist::{
    MAX_PLAYLIST_BYTES, MAX_PLAYLIST_LINE_BYTES, Playlist, PlaylistFormat, parse_playlist,
    write_m3u8,
};
pub use queue::{MAX_QUEUE_ITEMS, Queue, RepeatMode};

use fm_vfs::FmError;

fn error(message: impl Into<String>) -> FmError {
    FmError::backend("media", message)
}
