use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use fm_vfs::FmError;
use symphonia::core::{
    codecs::CodecParameters,
    codecs::audio::{
        AudioCodecParameters, AudioDecoder as CodecDecoder, AudioDecoderOptions,
        well_known::{CODEC_ID_AAC, CODEC_ID_ALAC},
    },
    common::Limit,
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType, probe::Hint},
    io::MediaSourceStream,
    meta::{MetadataOptions, MetadataRevision, RawValue, StandardTag, Tag, Visual},
    packet::Packet,
    units::{Time, TimeBase, Timestamp},
};

use crate::AudioFormat;
use crate::pipeline::MAX_BLOCK_FRAMES;

pub const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
const MAX_METADATA_BYTES: usize = 256 * 1024;
const MAX_TAG_CHARS: usize = 4096;

#[derive(Clone, Debug)]
pub struct CoverArt {
    pub media_type: String,
    /// Encoded image; image decoding must enforce its own pixel limit.
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct AudioTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub cover: Option<CoverArt>,
}

/// Tags of a metadata revision: the whole file's, then each track's.
fn revision_tags(metadata: &MetadataRevision) -> impl Iterator<Item = &Tag> {
    metadata
        .media
        .tags
        .iter()
        .chain(
            metadata
                .per_track
                .iter()
                .flat_map(|t| t.metadata.tags.iter()),
        )
        .take(256)
}

fn revision_visuals(metadata: &MetadataRevision) -> impl Iterator<Item = &Visual> {
    metadata
        .media
        .visuals
        .iter()
        .chain(
            metadata
                .per_track
                .iter()
                .flat_map(|t| t.metadata.visuals.iter()),
        )
        .take(16)
}

impl AudioTags {
    fn read(&mut self, metadata: &MetadataRevision) {
        for tag in revision_tags(metadata) {
            let (dst, value) = match &tag.std {
                Some(StandardTag::TrackTitle(value)) => (&mut self.title, value),
                Some(StandardTag::Artist(value)) => (&mut self.artist, value),
                Some(StandardTag::Album(value)) => (&mut self.album, value),
                Some(StandardTag::Genre(value)) => (&mut self.genre, value),
                _ => continue,
            };
            *dst = Some(value.chars().take(MAX_TAG_CHARS).collect());
        }
        if self.cover.is_none()
            && let Some(visual) = revision_visuals(metadata)
                .find(|v| !v.data.is_empty() && v.data.len() <= MAX_COVER_BYTES)
        {
            self.cover = Some(CoverArt {
                media_type: visual
                    .media_type
                    .as_deref()
                    .unwrap_or_default()
                    .chars()
                    .take(128)
                    .collect(),
                bytes: visual.data.to_vec(),
            });
        }
    }
}

#[derive(Clone, Debug)]
pub struct AudioInfo {
    pub format: AudioFormat,
    pub duration: Option<f64>,
    pub tags: AudioTags,
    /// Frames of encoder priming and padding already removed from the stream.
    pub encoder_delay: u32,
    pub encoder_padding: u32,
}

/// Valid sample window of an AAC/ALAC stream, from the iTunes `iTunSMPB` tag.
#[derive(Clone, Copy, Debug)]
struct Trim {
    delay: u64,
    valid: u64,
    padding: u32,
}

enum Trimmed {
    Keep(DecodedAudio),
    Before,
    After,
}

fn parse_smpb(value: &str) -> Option<Trim> {
    let mut fields = value
        .split_whitespace()
        .map(|f| u64::from_str_radix(f, 16).ok());
    fields.next()??;
    let delay = fields.next()??;
    let padding = fields.next()??;
    let valid = fields.next()??;
    (delay <= 100_000 && padding <= 100_000 && valid > 0).then_some(Trim {
        delay,
        valid,
        padding: padding as u32,
    })
}

fn read_smpb(metadata: &MetadataRevision) -> Option<Trim> {
    revision_tags(metadata)
        .find(|tag| tag.raw.key.ends_with("iTunSMPB"))
        .and_then(|tag| match &tag.raw.value {
            RawValue::String(value) => parse_smpb(value.get(..value.len().min(512))?),
            _ => None,
        })
}

/// One interleaved f32 packet with its media timestamp, after accurate-seek trimming.
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub position: f64,
}

/// Decodes one local audio file: in-process with Symphonia when it can read the format,
/// otherwise through an `ffmpeg` child (WMA, Opus, APE, AC-3…).
pub struct AudioDecoder {
    inner: Backend,
}

enum Backend {
    Symphonia(Box<SymphoniaDecoder>),
    Ffmpeg(Box<crate::ffmpeg_decoder::FfmpegDecoder>),
}

impl AudioDecoder {
    /// Probe a local file on a worker; no device is needed to decode it.
    pub fn open(path: &Path) -> Result<Self, FmError> {
        let inner = match SymphoniaDecoder::open(path) {
            Ok(decoder) => Backend::Symphonia(Box::new(decoder)),
            Err(error) => {
                // Regular files only; a missing file keeps its own (clearer) error.
                if !path.is_file() {
                    return Err(error);
                }
                Backend::Ffmpeg(Box::new(crate::ffmpeg_decoder::FfmpegDecoder::open(path)?))
            }
        };
        Ok(Self { inner })
    }

    pub fn info(&self) -> &AudioInfo {
        match &self.inner {
            Backend::Symphonia(decoder) => decoder.info(),
            Backend::Ffmpeg(decoder) => decoder.info(),
        }
    }

    pub fn seek(&mut self, position: f64) -> Result<(), FmError> {
        match &mut self.inner {
            Backend::Symphonia(decoder) => decoder.seek(position),
            Backend::Ffmpeg(decoder) => decoder.seek(position),
        }
    }

    pub fn next_packet(&mut self) -> Result<Option<DecodedAudio>, FmError> {
        match &mut self.inner {
            Backend::Symphonia(decoder) => decoder.next_packet(),
            Backend::Ffmpeg(decoder) => decoder.next_packet(),
        }
    }

    /// Whether ffmpeg decodes this file (it then has no embedded cover art).
    pub fn uses_ffmpeg(&self) -> bool {
        matches!(self.inner, Backend::Ffmpeg(_))
    }
}

struct SymphoniaDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn CodecDecoder>,
    track: u32,
    time_base: TimeBase,
    info: AudioInfo,
    seek_target: Option<f64>,
    decoded_any: bool,
    pending: Option<DecodedAudio>,
    decoded_end: f64,
    trim: Option<Trim>,
}

impl SymphoniaDecoder {
    fn open(path: &Path) -> Result<Self, FmError> {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(crate::error("Audio source must be a regular file"));
        }
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let metadata_options = MetadataOptions::default()
            .limit_tag_bytes(Limit::Maximum(MAX_METADATA_BYTES))
            .limit_visual_bytes(Limit::Maximum(MAX_COVER_BYTES));
        // Gapless trimming is always on in Symphonia 0.6 (format readers mark the trim,
        // decoders drop it).
        let mut reader = symphonia::default::get_probe()
            .probe(&hint, stream, FormatOptions::default(), metadata_options)
            .map_err(|e| crate::error(format!("Cannot recognize audio file: {e}")))?;
        // Revisions in reading order (trailing tags, leading tags, then the container's own);
        // later ones win, as before.
        let mut tags = AudioTags::default();
        let mut smpb = None;
        {
            let mut metadata = reader.metadata();
            for _ in 0..64 {
                if let Some(revision) = metadata.current() {
                    tags.read(revision);
                    smpb = read_smpb(revision).or(smpb);
                }
                if metadata.pop().is_none() {
                    break;
                }
            }
        }
        let track = reader
            .default_track(TrackType::Audio)
            .ok_or_else(|| crate::error("No audio track in file"))?;
        let Some(CodecParameters::Audio(params)) = &track.codec_params else {
            return Err(crate::error("No audio track in file"));
        };
        let declared_rate = params.sample_rate;
        let declared_time_base = track.time_base;
        let declared_duration = track.duration;
        let declared_frames = track.num_frames;
        let declared_delay = track.delay.unwrap_or(0);
        let declared_padding = track.padding.unwrap_or(0);
        // Symphonia trims MP3 and Vorbis itself; AAC and ALAC rely on the iTunSMPB tag.
        let trim = smpb.filter(|_| matches!(params.codec, CODEC_ID_AAC | CODEC_ID_ALAC));
        let params = &repair_aac_config(params.clone());
        let mut options = AudioDecoderOptions::default();
        options.gapless = true;
        options.verify = true;
        let mut decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &options)
            .map_err(|e| crate::error(format!("Cannot initialize audio decoder: {e}")))?;
        let track = track.id;
        // Some containers leave channel/rate information to the first audio packet.
        let packet = loop {
            match reader.next_packet() {
                Ok(Some(packet)) if packet.track_id == track => break packet,
                Ok(Some(_)) => continue,
                Ok(None) => return Err(crate::error("Audio file contains no audio packets")),
                Err(e) => {
                    return Err(crate::error(format!(
                        "Cannot read initial audio packet: {e}"
                    )));
                }
            }
        };
        let (format, initial) = decode_packet(decoder.as_mut(), &packet, None)
            .map_err(|e| crate::error(format!("Cannot decode initial audio packet: {e}")))?;
        let time_base = declared_time_base
            .or_else(|| TimeBase::try_from_recip(declared_rate.unwrap_or(format.rate())))
            .ok_or_else(|| crate::error("Audio track has no usable time base"))?;
        let duration = match trim {
            Some(trim) => Some(trim.valid as f64 / f64::from(format.rate())),
            None => declared_duration
                .map(|d| ticks_to_seconds(time_base, d.get() as f64))
                .or_else(|| declared_frames.map(|n| n as f64 / f64::from(format.rate()))),
        };
        let first_position = presented_at(time_base, &packet);
        let decoded_end = first_position
            + initial.len() as f64 / format.channels() as f64 / f64::from(format.rate());
        let mut this = Self {
            reader,
            decoder,
            track,
            time_base,
            info: AudioInfo {
                format,
                duration,
                tags,
                encoder_delay: trim.map_or(declared_delay, |t| t.delay as u32),
                encoder_padding: trim.map_or(declared_padding, |t| t.padding),
            },
            seek_target: None,
            decoded_any: !initial.is_empty(),
            pending: None,
            decoded_end,
            trim,
        };
        this.pending = match this.trimmed(first_position, initial) {
            Trimmed::Keep(packet) => Some(packet),
            Trimmed::Before | Trimmed::After => None,
        };
        Ok(this)
    }

    fn delay_seconds(&self) -> f64 {
        self.trim
            .map_or(0.0, |t| t.delay as f64 / f64::from(self.info.format.rate()))
    }

    /// Cut a raw-timeline packet to the valid window and move it to the trimmed timeline.
    fn trimmed(&self, position: f64, samples: Vec<f32>) -> Trimmed {
        trim_window(self.trim, self.info.format, position, samples)
    }

    pub fn info(&self) -> &AudioInfo {
        &self.info
    }

    pub fn seek(&mut self, position: f64) -> Result<(), FmError> {
        if !position.is_finite() || position < 0.0 {
            return Err(FmError::InvalidParams(
                "seek position must be finite and non-negative".into(),
            ));
        }
        let position =
            self.info.duration.map_or(position, |d| position.min(d)) + self.delay_seconds();
        self.reader
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time: Time::try_from_secs_f64(position)
                        .ok_or_else(|| crate::error("Seek position out of range"))?,
                    track_id: Some(self.track),
                },
            )
            .map_err(|e| crate::error(format!("Cannot seek audio: {e}")))?;
        self.decoder.reset();
        self.pending = None;
        self.decoded_end = position;
        self.seek_target = Some(position);
        Ok(())
    }

    pub fn next_packet(&mut self) -> Result<Option<DecodedAudio>, FmError> {
        if let Some(packet) = self.pending.take() {
            return Ok(Some(packet));
        }
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return self.ended(),
                Err(Error::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    return self.ended();
                }
                Err(err) => return Err(crate::error(format!("Cannot read audio packet: {err}"))),
            };
            if packet.track_id != self.track {
                continue;
            }
            let (format, buffer) =
                decode_packet(self.decoder.as_mut(), &packet, Some(self.info.format))?;
            self.decoded_any |= !buffer.is_empty();
            let mut position = presented_at(self.time_base, &packet);
            self.decoded_end = position
                + buffer.len() as f64 / format.channels() as f64 / f64::from(format.rate());
            let skip = self.seek_target.map_or(0, |target| {
                ((target - position).max(0.0) * f64::from(format.rate())).round() as usize
            });
            let skip = skip.min(buffer.len() / format.channels());
            if skip == buffer.len() / format.channels() {
                continue;
            }
            self.seek_target = None;
            position += skip as f64 / f64::from(format.rate());
            let samples = buffer
                .get(skip * format.channels()..)
                .ok_or_else(|| crate::error("Invalid decoded audio offset"))?
                .to_vec();
            match self.trimmed(position, samples) {
                Trimmed::Keep(packet) => return Ok(Some(packet)),
                Trimmed::Before => continue,
                Trimmed::After => return Ok(None),
            }
        }
    }
}

impl SymphoniaDecoder {
    /// End of stream: fine once every declared second has been decoded.
    fn ended(&self) -> Result<Option<DecodedAudio>, FmError> {
        if !self.decoded_any {
            return Err(crate::error("Audio file contains no complete samples"));
        }
        if self
            .info
            .duration
            .is_some_and(|duration| self.decoded_end - self.delay_seconds() + 0.1 < duration)
        {
            return Err(crate::error(
                "Audio file ended before its declared duration",
            ));
        }
        Ok(None)
    }
}

/// Decode one packet to interleaved f32 with every bound checked: encoded size, decoded
/// frames, a fixed format (when `expected` is given) and finite samples.
fn decode_packet(
    decoder: &mut dyn CodecDecoder,
    packet: &Packet,
    expected: Option<AudioFormat>,
) -> Result<(AudioFormat, Vec<f32>), FmError> {
    if packet.data.len() > 8 * 1024 * 1024 {
        return Err(crate::error("Encoded audio packet exceeds the size limit"));
    }
    let decoded = decoder
        .decode(packet)
        .map_err(|e| crate::error(format!("Cannot decode audio packet: {e}")))?;
    let spec = decoded.spec();
    let format = AudioFormat::new(spec.rate(), spec.channels().count())?;
    if decoded.frames() > MAX_BLOCK_FRAMES {
        return Err(crate::error("Decoded audio packet is too large"));
    }
    if expected.is_some_and(|expected| expected != format) {
        return Err(crate::error(
            "Audio format changed in the middle of the file",
        ));
    }
    let mut samples = Vec::with_capacity(decoded.samples_interleaved());
    decoded.copy_to_vec_interleaved(&mut samples);
    if samples.iter().any(|s: &f32| !s.is_finite()) {
        return Err(crate::error("Decoder returned non-finite audio samples"));
    }
    Ok((format, samples))
}

fn trim_window(
    trim: Option<Trim>,
    format: AudioFormat,
    position: f64,
    samples: Vec<f32>,
) -> Trimmed {
    let Some(trim) = trim else {
        return Trimmed::Keep(DecodedAudio { samples, position });
    };
    let channels = format.channels();
    let rate = f64::from(format.rate());
    let start = (position * rate).round().max(0.0) as u64;
    let end = start + (samples.len() / channels) as u64;
    let (low, high) = (trim.delay, trim.delay + trim.valid);
    if start >= high {
        return Trimmed::After;
    }
    if end <= low {
        return Trimmed::Before;
    }
    let from = low.saturating_sub(start) as usize;
    let to = (end.min(high) - start) as usize;
    let Some(kept) = samples.get(from * channels..to * channels) else {
        return Trimmed::Before;
    };
    Trimmed::Keep(DecodedAudio {
        samples: kept.to_vec(),
        position: ((start + from as u64) - trim.delay) as f64 / rate,
    })
}

/// When the packet's first decoded sample plays. `pts` counts from the first decoded frame,
/// encoder delay included (negative for MP3), and the decoder drops `trim_start` of them.
fn presented_at(time_base: TimeBase, packet: &Packet) -> f64 {
    let ts: Timestamp = packet.pts.saturating_add(packet.trim_start);
    ticks_to_seconds(time_base, ts.get() as f64)
}

/// Exact for sample-rate time bases (Symphonia's `Time` rounds to whole nanoseconds).
fn ticks_to_seconds(time_base: TimeBase, ticks: f64) -> f64 {
    ticks * f64::from(time_base.numer.get()) / f64::from(time_base.denom.get())
}

/// Some audiobook encoders write a 2-byte AAC config with the "depends on core coder" flag
/// set but no delay field. ffmpeg ignores it; Symphonia would fail with "end of bitstream".
fn repair_aac_config(mut params: AudioCodecParameters) -> AudioCodecParameters {
    if params.codec == CODEC_ID_AAC
        && let Some(config) = params.extra_data.as_mut()
        && config.len() == 2
    {
        config[1] &= !0x02;
    }
    params
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

    fn visual(bytes: usize, media_type: String) -> Visual {
        Visual {
            media_type: Some(media_type),
            dimensions: None,
            color_mode: None,
            usage: None,
            tags: Vec::new(),
            data: vec![1u8; bytes].into_boxed_slice(),
        }
    }

    #[test]
    fn hostile_tags_and_cover_art_are_capped_before_use() {
        use std::sync::Arc;
        use symphonia::core::meta::{METADATA_ID_NULL, MetadataBuilder, MetadataInfo};
        let info = MetadataInfo {
            metadata: METADATA_ID_NULL,
            short_name: "test",
            long_name: "test",
        };
        let long = Arc::new("x".repeat(MAX_TAG_CHARS * 10));
        let mut builder = MetadataBuilder::new(info);
        builder.add_tag(Tag::new_from_parts(
            "TIT2",
            RawValue::String(Arc::clone(&long)),
            Some(StandardTag::TrackTitle(long)),
        ));
        // An oversized picture is skipped; the next one that fits is used.
        builder.add_visual(visual(MAX_COVER_BYTES + 1, "image/png".into()));
        builder.add_visual(visual(16, "i".repeat(10_000)));
        let metadata = builder.build();
        let mut tags = AudioTags::default();
        tags.read(&metadata);
        assert_eq!(
            tags.title.as_deref().map(|t| t.chars().count()),
            Some(MAX_TAG_CHARS)
        );
        let cover = tags.cover.expect("the small picture");
        assert_eq!(cover.bytes.len(), 16);
        assert_eq!(cover.media_type.chars().count(), 128);

        let mut only_huge = MetadataBuilder::new(info);
        only_huge.add_visual(visual(MAX_COVER_BYTES + 1, "image/png".into()));
        let mut tags = AudioTags::default();
        tags.read(&only_huge.build());
        assert!(tags.cover.is_none(), "too large art is never kept");
    }

    #[test]
    fn itunsmpb_gives_delay_padding_and_length_and_rejects_nonsense() {
        let t = parse_smpb(" 00000000 00000840 000001CA 0000000000A3C1F0 00000000").expect("smpb");
        assert_eq!((t.delay, t.padding, t.valid), (0x840, 0x1CA, 0xA3_C1F0));
        for bad in ["", "zz", " 0 0 0 0", " 0 FFFFFFFF 0 10", " 0 1 2"] {
            assert!(parse_smpb(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn the_valid_window_drops_priming_and_padding_and_rebases_positions() {
        let format = AudioFormat::new(8_000, 1).expect("format");
        let trim = Some(Trim {
            delay: 10,
            valid: 20,
            padding: 5,
        });
        let packet = |start: u64, n: usize| {
            let samples = (0..n).map(|i| (start + i as u64) as f32).collect();
            trim_window(trim, format, start as f64 / 8_000.0, samples)
        };
        assert!(matches!(packet(0, 8), Trimmed::Before));
        let Trimmed::Keep(p) = packet(8, 8) else {
            panic!("keep")
        };
        assert_eq!((p.samples.first().copied(), p.position), (Some(10.0), 0.0));
        let Trimmed::Keep(p) = packet(24, 16) else {
            panic!("keep")
        };
        assert_eq!(p.samples.len(), 6);
        assert_eq!(p.samples.last().copied(), Some(29.0));
        assert!(matches!(packet(30, 8), Trimmed::After));
    }

    #[test]
    fn aac_config_with_a_dangling_core_coder_flag_is_repaired() {
        let mut params = AudioCodecParameters::new();
        params.for_codec(CODEC_ID_AAC);
        params.with_extra_data(vec![0x12, 0x12].into_boxed_slice());
        let fixed = repair_aac_config(params);
        assert_eq!(fixed.extra_data.as_deref(), Some(&[0x12, 0x10][..]));
        let mut long = AudioCodecParameters::new();
        long.for_codec(CODEC_ID_AAC);
        long.with_extra_data(vec![0x12, 0x12, 0x00].into_boxed_slice());
        assert_eq!(
            repair_aac_config(long).extra_data.as_deref(),
            Some(&[0x12, 0x12, 0x00][..])
        );
    }
}
