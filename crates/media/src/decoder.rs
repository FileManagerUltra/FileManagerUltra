use std::fs::File;
use std::io::ErrorKind;
use std::path::Path;

use fm_vfs::FmError;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{CODEC_TYPE_AAC, CODEC_TYPE_ALAC, Decoder, DecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::{Limit, MetadataOptions, MetadataRevision, StandardTagKey, Value},
    probe::Hint,
    units::{Time, TimeBase},
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

impl AudioTags {
    fn read(&mut self, metadata: &MetadataRevision) {
        for tag in metadata.tags().iter().take(256) {
            let dst = match tag.std_key {
                Some(StandardTagKey::TrackTitle) => &mut self.title,
                Some(StandardTagKey::Artist) => &mut self.artist,
                Some(StandardTagKey::Album) => &mut self.album,
                Some(StandardTagKey::Genre) => &mut self.genre,
                _ => continue,
            };
            if let Value::String(value) = &tag.value {
                *dst = Some(value.chars().take(MAX_TAG_CHARS).collect());
            }
        }
        if self.cover.is_none()
            && let Some(visual) = metadata
                .visuals()
                .iter()
                .take(16)
                .find(|v| !v.data.is_empty() && v.data.len() <= MAX_COVER_BYTES)
        {
            self.cover = Some(CoverArt {
                media_type: visual.media_type.chars().take(128).collect(),
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
    metadata
        .tags()
        .iter()
        .take(256)
        .find(|tag| tag.key.ends_with("iTunSMPB"))
        .and_then(|tag| match &tag.value {
            Value::String(value) => parse_smpb(value.get(..value.len().min(512))?),
            _ => None,
        })
}

/// One interleaved f32 packet with its media timestamp, after accurate-seek trimming.
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub position: f64,
}

pub struct AudioDecoder {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    time_base: TimeBase,
    info: AudioInfo,
    seek_target: Option<f64>,
    decoded_any: bool,
    pending: Option<DecodedAudio>,
    decoded_end: f64,
    trim: Option<Trim>,
}

impl AudioDecoder {
    /// Probe a local file on a worker; no device is needed to decode it.
    pub fn open(path: &Path) -> Result<Self, FmError> {
        let file = File::open(path)?;
        if !file.metadata()?.is_file() {
            return Err(crate::error("Audio source must be a regular file"));
        }
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut probed = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions {
                    limit_metadata_bytes: Limit::Maximum(MAX_METADATA_BYTES),
                    limit_visual_bytes: Limit::Maximum(MAX_COVER_BYTES),
                },
            )
            .map_err(|e| crate::error(format!("Cannot recognize audio file: {e}")))?;
        let mut tags = AudioTags::default();
        let mut smpb = None;
        if let Some(metadata) = probed.metadata.get()
            && let Some(revision) = metadata.current()
        {
            tags.read(revision);
            smpb = read_smpb(revision);
        }
        let mut reader = probed.format;
        if let Some(metadata) = reader.metadata().current() {
            tags.read(metadata);
            smpb = smpb.or_else(|| read_smpb(metadata));
        }
        let track = reader
            .default_track()
            .ok_or_else(|| crate::error("No audio track in file"))?;
        let params = &track.codec_params;
        let declared_rate = params.sample_rate;
        let declared_time_base = params.time_base;
        let declared_frames = params.n_frames;
        let declared_delay = params.delay.unwrap_or(0);
        let declared_padding = params.padding.unwrap_or(0);
        // Symphonia trims MP3 and Vorbis itself; AAC and ALAC rely on the iTunSMPB tag.
        let trim = smpb.filter(|_| matches!(params.codec, CODEC_TYPE_AAC | CODEC_TYPE_ALAC));
        let mut decoder = symphonia::default::get_codecs()
            .make(params, &DecoderOptions { verify: true })
            .map_err(|e| crate::error(format!("Cannot initialize audio decoder: {e}")))?;
        let track = track.id;
        // Some containers leave channel/rate information to the first audio packet.
        let packet = loop {
            let packet = reader
                .next_packet()
                .map_err(|e| crate::error(format!("Cannot read initial audio packet: {e}")))?;
            if packet.track_id() == track {
                break packet;
            }
        };
        if packet.data.len() > 8 * 1024 * 1024 {
            return Err(crate::error("Encoded audio packet exceeds the size limit"));
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| crate::error(format!("Cannot decode initial audio packet: {e}")))?;
        let spec = *decoded.spec();
        let format = AudioFormat::new(spec.rate, spec.channels.count())?;
        if decoded.capacity() > MAX_BLOCK_FRAMES {
            return Err(crate::error("Decoded audio packet is too large"));
        }
        let mut initial = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        initial.copy_interleaved_ref(decoded);
        if initial.samples().iter().any(|s| !s.is_finite()) {
            return Err(crate::error("Decoder returned non-finite audio samples"));
        }
        let time_base =
            declared_time_base.unwrap_or(TimeBase::new(1, declared_rate.unwrap_or(spec.rate)));
        let duration = match trim {
            Some(trim) => Some(trim.valid as f64 / f64::from(format.rate())),
            None => declared_frames.map(|n| seconds(time_base.calc_time(n))),
        };
        let first_position = seconds(time_base.calc_time(packet.ts()));
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
        this.pending = match this.trimmed(first_position, initial.samples().to_vec()) {
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
                    time: Time::from(position),
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
                Ok(packet) => packet,
                Err(Error::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    if !self.decoded_any {
                        return Err(crate::error("Audio file contains no complete samples"));
                    }
                    if self.info.duration.is_some_and(|duration| {
                        self.decoded_end - self.delay_seconds() + 0.1 < duration
                    }) {
                        return Err(crate::error(
                            "Audio file ended before its declared duration",
                        ));
                    }
                    return Ok(None);
                }
                Err(err) => return Err(crate::error(format!("Cannot read audio packet: {err}"))),
            };
            if packet.track_id() != self.track {
                continue;
            }
            if packet.data.len() > 8 * 1024 * 1024 {
                return Err(crate::error("Encoded audio packet exceeds the size limit"));
            }
            let decoded = self
                .decoder
                .decode(&packet)
                .map_err(|e| crate::error(format!("Cannot decode audio packet: {e}")))?;
            let spec = *decoded.spec();
            let format = AudioFormat::new(spec.rate, spec.channels.count())?;
            if format != self.info.format || decoded.capacity() > MAX_BLOCK_FRAMES {
                return Err(crate::error(
                    "Audio format changed or decoded packet is too large",
                ));
            }
            let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            buffer.copy_interleaved_ref(decoded);
            self.decoded_any |= !buffer.is_empty();
            let mut position = seconds(self.time_base.calc_time(packet.ts()));
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
                .samples()
                .get(skip * format.channels()..)
                .ok_or_else(|| crate::error("Invalid decoded audio offset"))?
                .to_vec();
            if samples.iter().any(|s| !s.is_finite()) {
                return Err(crate::error("Decoder returned non-finite audio samples"));
            }
            match self.trimmed(position, samples) {
                Trimmed::Keep(packet) => return Ok(Some(packet)),
                Trimmed::Before => continue,
                Trimmed::After => return Ok(None),
            }
        }
    }
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

fn seconds(time: Time) -> f64 {
    time.seconds as f64 + time.frac
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;

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
}
