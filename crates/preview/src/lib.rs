//! Turns a file into a plain [`Preview`] model: content sniffing, decoding, highlighting,
//! pretty-printing, images, hex and log parsing. No UI types and no threads of its own;
//! the engine runs [`render`] on a job lane. Everything read from disk is hostile:
//! sizes are capped, dimensions are checked from headers, and parsers are depth-limited.

pub mod arrange;
pub mod audio;
pub mod dir;
pub mod dvd_iso;
pub mod hex;
pub mod highlight;
pub mod image_prev;
pub mod log;
pub mod markdown;
pub mod platform;
pub mod player;
pub mod sniff;
pub mod structured;
pub mod subtitles;
pub mod svg;
pub mod text;
pub mod thumbs;
pub mod video;

use std::time::SystemTime;

use fm_vfs::{Entry, FmError, Vfs};

pub use highlight::{Class, Span};
pub use sniff::Kind;
pub use structured::Format;

/// Hard limits for one preview. Tests shrink them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Bytes read for text previews.
    pub text_bytes: usize,
    /// Highlighted lines kept.
    pub text_lines: usize,
    /// Largest file the Pretty toggle will parse.
    pub pretty_bytes: u64,
    /// Encoded image size.
    pub image_bytes: u64,
    /// Decoded pixels.
    pub image_pixels: u64,
    /// Longest edge of the stored RGBA preview.
    pub image_edge: u32,
    /// Animation frames kept.
    pub gif_frames: usize,
    /// Bytes shown in the hex dump.
    pub hex_bytes: usize,
    /// Directory entries counted.
    pub dir_entries: usize,
    /// Directory entries kept for the thumbnail grid.
    pub dir_items: usize,
    /// Entries visited by a recursive folder scan (files, folders and other nodes).
    pub dir_scan_entries: usize,
    /// Subfolder listings opened by a recursive folder scan.
    pub dir_scan_dirs: usize,
    /// Levels below the previewed folder a recursive scan descends.
    pub dir_scan_depth: usize,
    /// Wall-clock budget of a recursive folder scan.
    pub dir_scan_millis: u64,
}

impl Limits {
    /// A recursive scan that runs to completion (cancelled by the caller, not by a budget);
    /// the caps only exist to stop a pathological tree from running forever.
    pub fn unbounded_scan() -> Self {
        Self {
            dir_scan_entries: 100_000_000,
            dir_scan_dirs: 10_000_000,
            dir_scan_depth: 512,
            dir_scan_millis: 24 * 3600 * 1000,
            ..Self::default()
        }
    }
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            text_bytes: 1 << 20,
            text_lines: 20_000,
            pretty_bytes: 1 << 20,
            image_bytes: 64 << 20,
            image_pixels: 100_000_000,
            image_edge: 2048,
            gif_frames: 120,
            hex_bytes: 4096,
            dir_entries: 100_000,
            dir_items: 5_000,
            dir_scan_entries: 25_000,
            dir_scan_dirs: 2_000,
            dir_scan_depth: 32,
            dir_scan_millis: 4_000,
        }
    }
}

/// What is being previewed.
#[derive(Clone, Debug)]
pub struct Subject {
    pub entry: Entry,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextDoc {
    pub lines: Vec<Vec<Span>>,
    pub lang: String,
    /// The file is longer than what was read or kept.
    pub truncated: bool,
    pub total_bytes: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StructDoc {
    pub format: Format,
    pub raw: TextDoc,
    /// Re-indented form, or why it is unavailable (invalid, too large).
    pub pretty: Result<TextDoc, String>,
    /// Caveat shown next to the Pretty button (e.g. comments are dropped).
    pub pretty_note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageDoc {
    pub width: u32,
    pub height: u32,
    /// Frames of `width * height * 4` RGBA bytes with their delay in ms.
    pub frames: Vec<(Vec<u8>, u32)>,
    /// Source dimensions when the stored image was downscaled.
    pub original: (u32, u32),
    pub format: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HexDoc {
    pub bytes: Vec<u8>,
    pub total: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DirDoc {
    pub files: usize,
    pub dirs: usize,
    pub other: usize,
    pub bytes: u64,
    pub newest: Option<(String, SystemTime)>,
    pub truncated: bool,
    /// Visible children (folders first, then by name) for the thumbnail grid.
    /// When `recursive`, `name` is the path relative to the previewed folder.
    pub items: Vec<Entry>,
    /// More children exist than `items` holds.
    pub items_truncated: bool,
    /// The totals include every subfolder below the previewed one.
    pub recursive: bool,
    /// Subfolders whose listing could not be read (permission, vanished, dead share).
    pub unreadable: usize,
    pub oldest: Option<(String, SystemTime)>,
    pub largest: Option<(String, u64)>,
    pub hidden: usize,
    /// Recursive scans: each direct subfolder → (files, bytes) below it; at most `SUBDIRS_MAX`.
    pub subdirs: std::collections::BTreeMap<String, (usize, u64)>,
    /// A recursive scan is still running; the totals are partial.
    pub scanning: bool,
}

pub const SUBDIRS_MAX: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct MetaDoc {
    pub rows: Vec<(String, String)>,
    pub hint: Option<String>,
    /// A frame to show under the rows (video posters).
    pub poster: Option<Box<ImageDoc>>,
    /// Set when the video can be played in the pane.
    pub video: Option<VideoInfo>,
    /// Local audio metadata and playback source; loaded on an engine worker.
    pub audio: Option<audio::AudioDoc>,
}

/// What the player needs to know about a local video.
#[derive(Clone, Debug, PartialEq)]
pub struct VideoInfo {
    /// Absolute local path (checked again before it reaches ffmpeg).
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Seconds; 0 when unknown.
    pub duration: f64,
    /// Native format of the first audio stream, if present and supported.
    pub audio: Option<fm_media::AudioFormat>,
    /// Delay of the first audio stream relative to the container timeline.
    pub audio_delay: f64,
    /// Embedded audio and subtitle streams; empty when only `audio` is known.
    pub tracks: Tracks,
    /// Byte range of `path` that holds the video (a DVD image's main title), read through
    /// ffmpeg's `subfile` protocol; `None` plays the whole file.
    pub range: Option<(u64, u64)>,
}

/// Embedded streams of a video, in container order and capped at [`MAX_TRACKS`] each.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tracks {
    pub audio: Vec<AudioTrack>,
    pub subtitles: Vec<SubtitleTrack>,
    /// Audio stream decoded by the player: an index into `audio`.
    pub selected_audio: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioTrack {
    /// Display text from the stream's language, title and codec, sanitized and capped.
    pub label: String,
    /// `None` when the stream cannot be decoded, which makes it unselectable.
    pub format: Option<fm_media::AudioFormat>,
    pub delay: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubtitleTrack {
    pub label: String,
    /// Text codecs (SubRip, ASS, WebVTT, mov_text…) can be extracted and drawn;
    /// bitmap tracks (PGS, VobSub) cannot yet.
    pub text: bool,
    /// Container language tag, sanitized; `None` when missing or "und".
    pub language: Option<String>,
}

pub const MAX_TRACKS: usize = 16;

impl VideoInfo {
    /// The `-protocol_whitelist` and `-i` values that make ffmpeg read this video.
    pub fn ffmpeg_input(&self) -> Option<(&'static str, String)> {
        video::input_for(&self.path, self.range)
    }

    /// Format and start delay of the selected audio stream.
    pub fn audio_source(&self) -> Option<(fm_media::AudioFormat, f64)> {
        if self.tracks.audio.is_empty() {
            return self.audio.map(|format| (format, self.audio_delay));
        }
        let track = self.tracks.audio.get(self.tracks.selected_audio)?;
        Some((track.format?, track.delay))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Preview {
    Text(TextDoc),
    Structured(StructDoc),
    Markdown(markdown::MdDoc),
    Image(ImageDoc),
    Hex(HexDoc),
    Dir(DirDoc),
    Log(log::LogDoc),
    Meta(MetaDoc),
    Error(String),
}

/// Basic metadata rows shared by the cards.
pub fn base_meta(e: &Entry) -> Vec<(String, String)> {
    let mut rows = vec![("Name".to_string(), e.name.clone())];
    if let Some(s) = e.size {
        rows.push(("Size".into(), format!("{s} bytes")));
    }
    rows
}

/// Per-request choices that change the preview model (not the global limits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderOpts {
    /// Directory previews count and list every file below the folder.
    pub recursive: bool,
    /// Directory previews list hidden entries too.
    pub show_hidden: bool,
}

/// Preview `entry`, never panicking: a parser panic becomes an error card.
pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Preview {
    render_with(vfs, entry, limits, RenderOpts::default())
}

/// [`render`] with per-request options.
pub fn render_with(vfs: &dyn Vfs, entry: &Entry, limits: &Limits, opts: RenderOpts) -> Preview {
    render_with_log_limits(vfs, entry, limits, opts, &log::LogLimits::default())
}

/// Render with a shared-budget allocation for log windows in a multi-file preview.
pub fn render_with_log_limits(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    opts: RenderOpts,
    logs: &log::LogLimits,
) -> Preview {
    render_caught(vfs, entry, limits, opts, logs, Detection::Builtin)
}

/// Render using user filename associations, with content-only fallback when unmatched.
pub fn render_configured(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    opts: RenderOpts,
    logs: &log::LogLimits,
    association: Option<Kind>,
) -> Preview {
    render_caught(
        vfs,
        entry,
        limits,
        opts,
        logs,
        Detection::Configured(association),
    )
}

enum Detection {
    Builtin,
    Configured(Option<Kind>),
}

fn render_caught(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    opts: RenderOpts,
    logs: &log::LogLimits,
    detection: Detection,
) -> Preview {
    let run =
        std::panic::AssertUnwindSafe(|| render_inner(vfs, entry, limits, opts, logs, detection));
    match std::panic::catch_unwind(run) {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => Preview::Error(e.to_string()),
        Err(_) => Preview::Error("the preview failed to render this file".into()),
    }
}

fn render_inner(
    vfs: &dyn Vfs,
    entry: &Entry,
    limits: &Limits,
    opts: RenderOpts,
    logs: &log::LogLimits,
    detection: Detection,
) -> Result<Preview, FmError> {
    if entry.is_dir_like() {
        return dir::summarize_cancellable(
            vfs,
            entry,
            limits,
            opts.recursive,
            opts.show_hidden,
            None,
            None,
        )
        .map(Preview::Dir);
    }
    if !entry.is_file() && !entry.is_symlink() {
        return Ok(Preview::Meta(MetaDoc {
            rows: base_meta(entry),
            hint: Some("Special files are never opened.".into()),
            poster: None,
            video: None,
            audio: None,
        }));
    }
    let head = read_head(vfs, entry, 8192)?;
    let kind = match detection {
        Detection::Configured(association) => {
            sniff::configured(entry.uri.path_str(), &head, association)
        }
        Detection::Builtin => sniff::sniff(entry.uri.path_str(), &head),
    };
    match kind {
        Kind::Image(_) => image_prev::render(vfs, entry, limits),
        Kind::Log => log::open_doc_with(vfs, entry, limits, logs).map(Preview::Log),
        Kind::Text(lang) => text::render_text(vfs, entry, limits, &lang),
        Kind::Structured(f) => structured::render(vfs, entry, limits, f),
        Kind::Markdown => markdown::render(vfs, entry, limits),
        Kind::Video => Ok(video::render(entry, limits)),
        Kind::Audio => audio::render(entry, limits),
        Kind::Pdf => Ok(Preview::Meta(MetaDoc {
            rows: base_meta(entry),
            hint: Some(kind.hint().into()),
            poster: None,
            video: None,
            audio: None,
        })),
        Kind::Binary => hex::render(vfs, entry, limits).map(Preview::Hex),
    }
}

/// Read up to `n` bytes from the start.
pub fn read_head(vfs: &dyn Vfs, entry: &Entry, n: usize) -> Result<Vec<u8>, FmError> {
    use std::io::Read;
    let mut r = vfs.open_read(&entry.uri)?;
    let mut buf = Vec::with_capacity(n.min(1 << 16));
    r.by_ref().take(n as u64).read_to_end(&mut buf)?;
    Ok(buf)
}
