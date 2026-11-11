//! Fallback audio decoder for formats Symphonia cannot read (WMA, Opus, APE, AC-3…):
//! `ffprobe` for the format and tags, then an `ffmpeg` child that pipes interleaved f32
//! PCM. A reader thread with a bounded channel keeps memory flat, and every wait has a
//! timeout, so a hung decoder reports an error instead of stalling playback. The child is
//! killed when the decoder is dropped or seeks.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, RecvTimeoutError, sync_channel};
use std::time::{Duration, Instant};

use fm_vfs::FmError;

use crate::AudioFormat;
use crate::decoder::{AudioInfo, AudioTags, DecodedAudio};

/// Finds `ffmpeg` / `ffprobe`; registered by the app (it knows the managed tools folder).
static LOCATOR: OnceLock<fn(&str) -> Option<PathBuf>> = OnceLock::new();

/// Register how to find ffmpeg tools. The first registration wins.
pub fn set_tool_locator(locate: fn(&str) -> Option<PathBuf>) {
    let _ = LOCATOR.set(locate);
}

fn tool(name: &str) -> Option<PathBuf> {
    LOCATOR.get().and_then(|locate| locate(name))
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// No PCM for this long while the decoder is running means it hangs.
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_BYTES: u64 = 64 * 1024;
/// Frames per packet handed to the pipeline (well under its block limit).
const PACKET_FRAMES: usize = 4096;
/// Packets buffered ahead of the player: about 1.5 s at 44.1 kHz.
const AHEAD: usize = 16;

fn command(tool: &Path) -> Command {
    let mut c = Command::new(tool);
    c.stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// An absolute UTF-8 path that cannot be read as an option or a protocol URL.
fn safe_path(path: &Path) -> Option<&str> {
    let text = path.to_str()?;
    let absolute =
        text.starts_with('/') || text.get(1..3) == Some(":\\") || text.starts_with("\\\\");
    (absolute && !text.contains('\0')).then_some(text)
}

fn unsupported() -> FmError {
    crate::error("This audio format is not supported (installing ffmpeg adds more formats)")
}

/// Run ffprobe with a deadline; `key=value` lines of the first audio stream and format.
fn probe(path: &str) -> Result<AudioInfo, FmError> {
    let ffprobe = tool("ffprobe").ok_or_else(unsupported)?;
    let mut c = command(&ffprobe);
    c.args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-select_streams",
        "a:0",
        "-show_entries",
        // Ogg/Opus keep tags on the stream, ASF/Matroska on the container; the
        // format section prints last, so container tags win when both exist.
        "stream=sample_rate,channels:stream_tags=title,artist,album,genre:format=duration:format_tags=title,artist,album,genre",
        "-of",
        "default=noprint_wrappers=1",
        "-i",
    ])
    .arg(path);
    let mut child = c.spawn().map_err(|_| unsupported())?;
    let mut out = child.stdout.take().ok_or_else(unsupported)?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut out).take(PROBE_BYTES).read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    if !ok {
        return Err(crate::error("ffmpeg could not read this audio file"));
    }
    parse_probe(&text)
}

fn parse_probe(text: &str) -> Result<AudioInfo, FmError> {
    let mut rate = None;
    let mut channels = None;
    let mut duration = None;
    let mut tags = AudioTags::default();
    let clean = |value: &str| -> Option<String> {
        let value: String = value
            .chars()
            .filter(|c| !c.is_control())
            .take(512)
            .collect();
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    };
    for line in text.lines().take(256) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.to_ascii_lowercase().as_str() {
            "sample_rate" => rate = value.trim().parse::<u32>().ok(),
            "channels" => channels = value.trim().parse::<usize>().ok(),
            "duration" => {
                duration = value
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|d| d.is_finite() && *d > 0.0 && *d < 1e7)
            }
            "tag:title" => tags.title = clean(value),
            "tag:artist" => tags.artist = clean(value),
            "tag:album" => tags.album = clean(value),
            "tag:genre" => tags.genre = clean(value),
            _ => {}
        }
    }
    let (Some(rate), Some(channels)) = (rate, channels) else {
        return Err(crate::error("No audio track in file"));
    };
    Ok(AudioInfo {
        format: AudioFormat::new(rate, channels)?,
        duration,
        tags,
        encoder_delay: 0,
        encoder_padding: 0,
    })
}

enum Chunk {
    Pcm(Vec<f32>),
    End,
}

pub(crate) struct FfmpegDecoder {
    path: String,
    info: AudioInfo,
    child: Option<Child>,
    receiver: Option<Receiver<Chunk>>,
    /// Media time of the next packet.
    position: f64,
    decoded_any: bool,
}

impl FfmpegDecoder {
    pub(crate) fn open(path: &Path) -> Result<Self, FmError> {
        let text = safe_path(path).ok_or_else(|| crate::error("Choose a safe local audio path"))?;
        let info = probe(text)?;
        let mut this = Self {
            path: text.to_string(),
            info,
            child: None,
            receiver: None,
            position: 0.0,
            decoded_any: false,
        };
        this.spawn(0.0)?;
        Ok(this)
    }

    pub(crate) fn info(&self) -> &AudioInfo {
        &self.info
    }

    fn stop(&mut self) {
        self.receiver = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn spawn(&mut self, from: f64) -> Result<(), FmError> {
        self.stop();
        let ffmpeg = tool("ffmpeg").ok_or_else(unsupported)?;
        let format = self.info.format;
        let mut c = command(&ffmpeg);
        c.args([
            "-v",
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file",
            "-ss",
        ])
        .arg(format!("{from:.6}"))
        .arg("-i")
        .arg(&self.path)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-f",
            "f32le",
            "-c:a",
            "pcm_f32le",
        ])
        .args(["-ac", &format.channels().to_string()])
        .args(["-ar", &format.rate().to_string()])
        .arg("-");
        let mut child = c
            .spawn()
            .map_err(|_| crate::error("Could not start ffmpeg to decode audio"))?;
        let mut out = child
            .stdout
            .take()
            .ok_or_else(|| crate::error("Could not read from ffmpeg"))?;
        let (tx, receiver) = sync_channel(AHEAD);
        let frame_bytes = format.channels() * 4;
        std::thread::Builder::new()
            .name("fm-audio-ffmpeg".into())
            .spawn(move || {
                let mut buffer = vec![0u8; PACKET_FRAMES * frame_bytes];
                let mut filled = 0;
                loop {
                    let read = match out.read(&mut buffer[filled..]) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    filled += read;
                    if filled == buffer.len() {
                        if tx.send(Chunk::Pcm(samples(&buffer))).is_err() {
                            return;
                        }
                        filled = 0;
                    }
                }
                let whole = filled / frame_bytes * frame_bytes;
                if whole > 0 {
                    let _ = tx.send(Chunk::Pcm(samples(&buffer[..whole])));
                }
                let _ = tx.send(Chunk::End);
            })
            .map_err(FmError::Io)?;
        self.child = Some(child);
        self.receiver = Some(receiver);
        self.position = from;
        Ok(())
    }

    pub(crate) fn seek(&mut self, position: f64) -> Result<(), FmError> {
        if !position.is_finite() || position < 0.0 {
            return Err(FmError::InvalidParams(
                "seek position must be finite and non-negative".into(),
            ));
        }
        let position = self.info.duration.map_or(position, |d| position.min(d));
        self.spawn(position)
    }

    pub(crate) fn next_packet(&mut self) -> Result<Option<DecodedAudio>, FmError> {
        let Some(receiver) = &self.receiver else {
            return Ok(None);
        };
        match receiver.recv_timeout(STALL_TIMEOUT) {
            Ok(Chunk::Pcm(samples)) => {
                if samples.iter().any(|s| !s.is_finite()) {
                    return Err(crate::error("Decoder returned non-finite audio samples"));
                }
                let frames = samples.len() / self.info.format.channels();
                let position = self.position;
                self.position += frames as f64 / f64::from(self.info.format.rate());
                self.decoded_any = true;
                Ok(Some(DecodedAudio { samples, position }))
            }
            Ok(Chunk::End) | Err(RecvTimeoutError::Disconnected) => {
                let success = self
                    .child
                    .take()
                    .and_then(|mut child| child.wait().ok())
                    .is_some_and(|status| status.success());
                self.receiver = None;
                if !success || !self.decoded_any {
                    return Err(crate::error("ffmpeg could not decode this audio file"));
                }
                Ok(None)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.stop();
                Err(crate::error("The audio decoder stopped responding"))
            }
        }
    }
}

impl Drop for FfmpegDecoder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn samples(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_parses_and_hostile_values_are_bounded() {
        let info = parse_probe(
            "sample_rate=44100\nchannels=2\nduration=640.917000\nTAG:title=High\u{7}Society\nTAG:artist=Toon Hermans\n",
        )
        .expect("info");
        assert_eq!((info.format.rate(), info.format.channels()), (44_100, 2));
        assert_eq!(info.duration, Some(640.917));
        assert_eq!(info.tags.title.as_deref(), Some("HighSociety"));
        assert_eq!(info.tags.artist.as_deref(), Some("Toon Hermans"));
        for bad in [
            "",
            "sample_rate=44100\n",
            "sample_rate=0\nchannels=2\n",
            "sample_rate=44100\nchannels=0\n",
            "sample_rate=x\nchannels=2\n",
        ] {
            assert!(parse_probe(bad).is_err(), "{bad:?}");
        }
        let odd = parse_probe("sample_rate=8000\nchannels=1\nduration=inf\n").expect("odd");
        assert_eq!(odd.duration, None);
    }

    #[test]
    fn unsafe_paths_are_refused() {
        assert!(safe_path(Path::new("relative.wma")).is_none());
        assert!(safe_path(Path::new("-i")).is_none());
        assert!(safe_path(Path::new("/music/a.wma")).is_some());
    }
}
