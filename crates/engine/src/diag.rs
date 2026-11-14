//! Diagnostic log (capability `diagnostics`): an opt-in, owner-only, size-capped text log of
//! memory samples and what the app did, for chasing leaks on a user's machine.
//!
//! Hooks anywhere in the app call [`count`] or [`event`]; with the log off they cost one
//! atomic load. With it on, events go through a bounded channel to a writer thread, which
//! does all file I/O and samples process memory once per second, so the UI thread never
//! blocks. Events carry command ids, numbers and fixed labels only: never paths, names,
//! URIs, parameters or credentials (requirement "Diagnostic log is private and bounded").

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError, bounded};
use fm_vfs::FmError;

/// A log file stops growing at this size.
pub const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
/// Event lines written per second at most; the rest are counted in the next sample.
pub const MAX_EVENTS_PER_SEC: u32 = 200;
/// Log files kept in the folder, the new one included.
pub const KEEP_LOGS: usize = 20;
/// Records queued for the writer; a full queue drops (and counts) instead of blocking.
const QUEUE: usize = 4096;
/// Graphics-stack records kept from startup, so turning the log on later still names the
/// OpenGL renderer.
const STARTUP_RECORDS: usize = 64;
/// One event line is cut to this many bytes.
const MAX_LINE: usize = 512;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// A log was started in [`default_dir`] in this process (the folder exists).
static STARTED: AtomicBool = AtomicBool::new(false);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static STARTUP: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Milliseconds since [`epoch`] of the last UI sample.
static LAST_UI_SAMPLE: AtomicU64 = AtomicU64::new(0);

/// Things counted while the log is on; the sample line shows them per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counter {
    /// UI frames drawn.
    Frames,
    /// New video frames taken from a decoder.
    VideoFrames,
    /// Video textures created (`load_texture`).
    TexCreated,
    /// Video textures given new storage (`set` with a new size).
    TexResized,
    /// Video frames written into an existing texture (`set_partial`).
    TexInPlace,
    /// Frame-cache entries released because they were not drawn.
    CacheEvicted,
    /// Video textures among them.
    VideoTexEvicted,
    /// Video decoders started.
    DecoderStarts,
    /// Commands dispatched.
    Commands,
}

const COUNTERS: usize = 9;
const COUNTER_NAMES: [&str; COUNTERS] = [
    "frames",
    "vframes",
    "tex_new",
    "tex_resize",
    "tex_inplace",
    "evict",
    "vtex_evict",
    "dec_start",
    "cmds",
];
static COUNTS: [AtomicU64; COUNTERS] = [const { AtomicU64::new(0) }; COUNTERS];

/// What the UI thread knows, sent once per second ([`ui_sample_due`], [`send_ui_sample`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiSample {
    pub textures: usize,
    pub texture_bytes: u64,
    pub pixels_per_point: f32,
    pub panes: usize,
    pub workspaces: usize,
    pub previews: usize,
    pub terminals: usize,
    pub thumb_bytes: u64,
    pub live_players: usize,
    /// Size of the newest frame of each video on screen.
    pub videos: Vec<(u32, u32)>,
    pub texture_reuse: bool,
}

enum Msg {
    Line(String),
    Ui(UiSample),
}

struct Running {
    tx: Sender<Msg>,
    stop: Sender<()>,
    path: PathBuf,
    thread: Option<std::thread::JoinHandle<()>>,
}

fn lock<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    // A poisoned lock only means a writer thread panicked mid-update; the data is still usable.
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// Whether the log is being written. Hooks may check this before preparing an event.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Count one occurrence of `counter` (no-op while the log is off).
pub fn count(counter: Counter) {
    if enabled() {
        COUNTS[counter as usize].fetch_add(1, Ordering::Relaxed);
    }
}

/// Write an event line `kind: detail`. `detail` is built only while the log is on and must
/// not contain paths, names, URIs, parameters or credentials.
pub fn event(kind: &'static str, detail: impl FnOnce() -> String) {
    if !enabled() {
        return;
    }
    let mut line = format!("{kind}: {}", detail());
    truncate(&mut line, MAX_LINE);
    send(Msg::Line(line));
}

fn send(msg: Msg) {
    let guard = lock(&RUNNING);
    let Some(running) = guard.as_ref() else {
        return;
    };
    match running.tx.try_send(msg) {
        Ok(()) => {}
        Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn truncate(line: &mut String, max: usize) {
    if line.len() > max {
        let mut cut = max;
        while !line.is_char_boundary(cut) {
            cut -= 1;
        }
        line.truncate(cut);
        line.push('…');
    }
}

/// True at most once per second while the log is on: the UI thread then builds a
/// [`UiSample`] and hands it to [`send_ui_sample`].
pub fn ui_sample_due() -> bool {
    if !enabled() {
        return false;
    }
    let now = u64::try_from(epoch().elapsed().as_millis()).unwrap_or(u64::MAX);
    let last = LAST_UI_SAMPLE.load(Ordering::Relaxed);
    now.saturating_sub(last) >= 1000
        && LAST_UI_SAMPLE
            .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

/// Fill a [`UiSample`] from the session; the caller adds what only the UI knows.
pub fn session_sample(
    s: &crate::Session,
    textures: usize,
    texture_bytes: u64,
    pixels_per_point: f32,
) -> UiSample {
    let mut videos: Vec<(u32, u32)> = s
        .previews
        .slots
        .values()
        .filter_map(|slot| slot.video.as_ref()?.frame.as_ref())
        .map(|f| (f.width, f.height))
        .collect();
    if let Some(f) = s.media.video.as_ref().and_then(|v| v.frame.as_ref()) {
        videos.push((f.width, f.height));
    }
    UiSample {
        textures,
        texture_bytes,
        pixels_per_point,
        panes: s.layout.panes().len(),
        workspaces: s.workspaces.tabs.len(),
        previews: s.previews.slots.len(),
        terminals: s.terminals.len(),
        thumb_bytes: s.thumbs.memory_bytes() as u64,
        live_players: fm_preview::player::live_players(),
        videos,
        texture_reuse: s.prefs.video_texture_reuse,
    }
}

pub fn send_ui_sample(sample: UiSample) {
    if enabled() {
        send(Msg::Ui(sample));
    }
}

/// `<data dir>/diagnostics`, next to the managed tools.
pub fn default_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", fm_brand::APP_SLUG)
        .map(|dirs| dirs.data_local_dir().join("diagnostics"))
}

/// Whether the log folder exists: a log was written this session or before. The folder is
/// looked up once; later starts set the flag, so menus never touch the disk per frame.
pub fn has_logs() -> bool {
    static EXISTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    STARTED.load(Ordering::Relaxed)
        || *EXISTED.get_or_init(|| default_dir().is_some_and(|dir| dir.is_dir()))
}

/// The file being written, while the log is on.
pub fn current_path() -> Option<PathBuf> {
    lock(&RUNNING).as_ref().map(|r| r.path.clone())
}

/// Start writing a new log in `dir` with `header` lines first (version, build, settings).
/// Starting while running is a no-op that returns the current file.
pub fn start(dir: &Path, header: Vec<String>) -> Result<PathBuf, FmError> {
    let mut guard = lock(&RUNNING);
    if let Some(running) = guard.as_ref() {
        return Ok(running.path.clone());
    }
    create_private_dir(dir).map_err(FmError::Io)?;
    prune(dir, KEEP_LOGS.saturating_sub(1));
    let stamp = jiff::Zoned::now().strftime("%Y%m%d-%H%M%S").to_string();
    let path = dir.join(format!("diagnostics-{stamp}.log"));
    let file = open_private(&path).map_err(FmError::Io)?;
    for c in &COUNTS {
        c.store(0, Ordering::Relaxed);
    }
    DROPPED.store(0, Ordering::Relaxed);
    let (tx, rx) = bounded(QUEUE);
    let (stop, stop_rx) = bounded(1);
    let mut lines = header;
    lines.extend(system_lines());
    {
        let startup = lock(&STARTUP);
        if !startup.is_empty() {
            lines.push("graphics stack at startup:".into());
            lines.extend(startup.iter().map(|l| format!("  {l}")));
        }
    }
    let thread = std::thread::Builder::new()
        .name("diagnostics".into())
        .spawn(move || writer(file, lines, rx, stop_rx))
        .map_err(|e| FmError::backend("diagnostics", format!("Cannot start the log: {e}")))?;
    *guard = Some(Running {
        tx,
        stop,
        path: path.clone(),
        thread: Some(thread),
    });
    ENABLED.store(true, Ordering::Relaxed);
    log::set_max_level(log::LevelFilter::Debug);
    Ok(path)
}

/// Stop writing; the file is flushed and closed. A no-op while not running.
pub fn stop() {
    ENABLED.store(false, Ordering::Relaxed);
    let running = lock(&RUNNING).take();
    if let Some(mut running) = running {
        let _ = running.stop.try_send(());
        drop(running.tx);
        if let Some(thread) = running.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Follow the preference: start (in [`default_dir`]) or stop. Returns the error message
/// when the log could not start.
pub fn apply(on: bool, header: impl FnOnce() -> Vec<String>) -> Result<(), FmError> {
    if on == enabled() {
        return Ok(());
    }
    if !on {
        stop();
        return Ok(());
    }
    let dir = default_dir()
        .ok_or_else(|| FmError::backend("diagnostics", "No data folder for the log"))?;
    start(&dir, header())?;
    STARTED.store(true, Ordering::Relaxed);
    Ok(())
}

/// The build commit of release-workflow builds (`GITHUB_SHA` at compile time), else "local".
pub fn build_commit() -> &'static str {
    match option_env!("GITHUB_SHA") {
        Some(sha) => sha.get(..12).unwrap_or(sha),
        None => "local",
    }
}

/// Version, build and the settings that change how video is decoded and drawn.
pub fn header(prefs: &crate::prefs::Prefs) -> Vec<String> {
    vec![
        format!(
            "version: {} build: {}",
            env!("CARGO_PKG_VERSION"),
            build_commit()
        ),
        format!(
            "settings: hardware_decoding={} ({}) video_texture_reuse={} zoom={}",
            prefs.media_hardware_decoding,
            fm_preview::player::HwAccel::for_platform().label(),
            prefs.video_texture_reuse,
            prefs.zoom_factor(),
        ),
    ]
}

fn system_lines() -> Vec<String> {
    let mut lines = vec![format!(
        "os: {} ({}), kernel {}, arch {}",
        sysinfo::System::long_os_version().unwrap_or_else(|_| "unknown".into()),
        std::env::consts::OS,
        sysinfo::System::kernel_long_version().unwrap_or_else(|_| "unknown".into()),
        std::env::consts::ARCH,
    )];
    if let Ok(mut sys) = sysinfo::System::new() {
        sys.refresh_memory();
        sys.refresh_cpu_list(sysinfo::CpuRefreshKind::nothing());
        let cpu = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .unwrap_or_default();
        lines.push(format!(
            "machine: {} logical CPUs ({cpu}), {} RAM",
            sys.cpus().len(),
            mib(sys.total_memory()),
        ));
    }
    lines
}

fn mib(bytes: u64) -> String {
    format!("{:.1}M", bytes as f64 / 1_048_576.0)
}

struct Process {
    sys: Option<sysinfo::System>,
    pid: Option<sysinfo::Pid>,
}

impl Process {
    fn new() -> Self {
        Self {
            sys: sysinfo::System::new().ok(),
            pid: sysinfo::get_current_pid().ok(),
        }
    }

    /// `(working set, commit/private bytes, dedicated GPU memory)`.
    fn memory(&mut self) -> Option<(u64, u64, Option<u64>)> {
        let pid = self.pid?;
        let sys = self.sys.as_mut()?;
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            false,
            sysinfo::ProcessRefreshKind::nothing()
                .with_memory()
                .with_gpu_memory(),
        );
        let p = sys.process(pid)?;
        Some((p.memory(), p.virtual_memory(), p.gpu_memory()))
    }
}

/// Output side of the writer: byte cap and line formatting.
struct Out<W: std::io::Write> {
    file: W,
    written: u64,
    full: bool,
    start: Instant,
}

impl<W: std::io::Write> Out<W> {
    fn line(&mut self, text: &str) {
        if self.full {
            return;
        }
        let stamp = jiff::Zoned::now().strftime("%H:%M:%S%.3f").to_string();
        let t = self.start.elapsed().as_secs_f64();
        let line = format!("{stamp} {t:9.3} {text}\n");
        let end = "log size limit reached; nothing more is written\n";
        if self.written + (line.len() + end.len()) as u64 > MAX_LOG_BYTES {
            let _ = self.file.write_all(end.as_bytes());
            let _ = self.file.flush();
            self.full = true;
            return;
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.written += line.len() as u64;
        }
    }
}

fn writer(file: std::fs::File, header: Vec<String>, rx: Receiver<Msg>, stop: Receiver<()>) {
    let mut out = Out {
        file: std::io::BufWriter::new(file),
        written: 0,
        full: false,
        start: Instant::now(),
    };
    out.line(&format!(
        "{} diagnostic log; sample columns are per second unless marked total",
        fm_brand::APP_NAME
    ));
    for line in &header {
        out.line(line);
    }
    let _ = out.file.flush();
    let mut state = WriterState::default();
    let mut process = Process::new();
    let mut next = Instant::now() + Duration::from_secs(1);
    loop {
        if stop.try_recv().is_ok() {
            break;
        }
        let wait = next.saturating_duration_since(Instant::now());
        match rx.recv_timeout(wait) {
            Ok(Msg::Line(text)) => state.event(&mut out, text),
            Ok(Msg::Ui(sample)) => state.ui = Some(sample),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if Instant::now() >= next {
            let memory = process.memory();
            state.sample(&mut out, memory);
            let _ = out.file.flush();
            next += Duration::from_secs(1);
            // After a stall (sleep, debugger) do not write a burst of catch-up samples.
            if next < Instant::now() {
                next = Instant::now() + Duration::from_secs(1);
            }
        }
    }
    state.flush_repeat(&mut out);
    out.line("log stopped");
    let _ = out.file.flush();
}

#[derive(Default)]
struct WriterState {
    ui: Option<UiSample>,
    events_this_second: u32,
    suppressed: u64,
    last_event: Option<String>,
    repeats: u64,
    previous: [u64; COUNTERS],
    dropped_seen: u64,
}

impl WriterState {
    fn event<W: std::io::Write>(&mut self, out: &mut Out<W>, text: String) {
        if self.last_event.as_deref() == Some(text.as_str()) {
            self.repeats += 1;
            return;
        }
        self.flush_repeat(out);
        if self.events_this_second >= MAX_EVENTS_PER_SEC {
            self.suppressed += 1;
            return;
        }
        self.events_this_second += 1;
        out.line(&text);
        self.last_event = Some(text);
    }

    fn flush_repeat<W: std::io::Write>(&mut self, out: &mut Out<W>) {
        if self.repeats > 0 {
            out.line(&format!(
                "  (previous line repeated {} times)",
                self.repeats
            ));
            self.repeats = 0;
        }
    }

    fn sample<W: std::io::Write>(
        &mut self,
        out: &mut Out<W>,
        memory: Option<(u64, u64, Option<u64>)>,
    ) {
        self.flush_repeat(out);
        self.last_event = None;
        let mut line = String::from("sample");
        match memory {
            Some((ws, commit, gpu)) => {
                line.push_str(&format!(" ws={} commit={}", mib(ws), mib(commit)));
                match gpu {
                    Some(gpu) => line.push_str(&format!(" gpu={}", mib(gpu))),
                    None => line.push_str(" gpu=n/a"),
                }
            }
            None => line.push_str(" mem=n/a"),
        }
        if let Some(ui) = &self.ui {
            line.push_str(&format!(
                " tex={} texmem={} players={} panes={} workspaces={} previews={} terms={} thumbs={} ppp={:.2} reuse={}",
                ui.textures,
                mib(ui.texture_bytes),
                ui.live_players,
                ui.panes,
                ui.workspaces,
                ui.previews,
                ui.terminals,
                mib(ui.thumb_bytes),
                ui.pixels_per_point,
                if ui.texture_reuse { "on" } else { "off" },
            ));
            if !ui.videos.is_empty() {
                let sizes: Vec<String> =
                    ui.videos.iter().map(|(w, h)| format!("{w}x{h}")).collect();
                line.push_str(&format!(" video={}", sizes.join(",")));
            }
        }
        for (i, name) in COUNTER_NAMES.iter().enumerate() {
            let now = COUNTS[i].load(Ordering::Relaxed);
            let delta = now.saturating_sub(self.previous[i]);
            self.previous[i] = now;
            line.push_str(&format!(" {name}={delta}"));
        }
        line.push_str(&format!(
            " tex_new_total={} dec_start_total={}",
            COUNTS[Counter::TexCreated as usize].load(Ordering::Relaxed)
                + COUNTS[Counter::TexResized as usize].load(Ordering::Relaxed),
            COUNTS[Counter::DecoderStarts as usize].load(Ordering::Relaxed),
        ));
        let dropped = DROPPED.load(Ordering::Relaxed);
        if self.suppressed > 0 || dropped > self.dropped_seen {
            line.push_str(&format!(
                " events_suppressed={} queue_dropped={}",
                self.suppressed,
                dropped.saturating_sub(self.dropped_seen)
            ));
        }
        self.dropped_seen = dropped;
        self.suppressed = 0;
        self.events_this_second = 0;
        out.line(&line);
    }
}

/// Delete the oldest `diagnostics-*.log` files so at most `keep` remain.
fn prune(dir: &Path, keep: usize) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = read
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("diagnostics-") && n.ends_with(".log"))
        })
        .take(10_000)
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort();
    let excess = logs.len() - keep;
    for old in logs.into_iter().take(excess) {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    // Windows: the per-user profile's ACLs are inherited.
    std::fs::create_dir_all(dir)
}

fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let file = options.open(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
    #[cfg(not(unix))]
    options.open(path)
}

// ---- `log` crate sink -------------------------------------------------------------------

struct Sink;
static SINK: Sink = Sink;

/// Only the graphics stack is listened to: its records name the OpenGL driver and report
/// texture and context trouble, and carry no user data. Network crates are never accepted.
fn accepted(target: &str, level: log::Level) -> bool {
    let crate_name = target.split("::").next().unwrap_or(target);
    match crate_name {
        "egui_glow" => level <= log::Level::Debug,
        "eframe" | "egui_winit" | "winit" | "glutin" | "glutin_winit" => level <= log::Level::Warn,
        _ => false,
    }
}

impl log::Log for Sink {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        accepted(metadata.target(), metadata.level())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !accepted(record.target(), record.level()) {
            return;
        }
        let mut text = format!("{} {}: {}", record.level(), record.target(), record.args())
            .replace('\n', " | ");
        truncate(&mut text, MAX_LINE);
        if enabled() {
            send(Msg::Line(format!("log: {text}")));
        } else {
            let mut startup = lock(&STARTUP);
            if startup.len() < STARTUP_RECORDS {
                startup.push(text);
            }
        }
    }

    fn flush(&self) {}
}

/// Listen to the graphics stack's `log` records from now on (call once, before the window
/// opens). Records arriving before the log is turned on are kept (bounded) for its header.
pub fn install_log_sink() {
    if log::set_logger(&SINK).is_ok() {
        log::set_max_level(log::LevelFilter::Debug);
    }
}

/// Startup is over: while the log is off, stop paying for debug-level records.
pub fn startup_done() {
    if !enabled() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log is process-global; tests that start it run one at a time.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn serial() -> MutexGuard<'static, ()> {
        SERIAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn off_by_default_hooks_do_nothing_and_create_nothing() {
        let _g = serial();
        assert!(!enabled());
        let before = COUNTS[Counter::Commands as usize].load(Ordering::Relaxed);
        count(Counter::Commands);
        let mut built = false;
        event("cmd", || {
            built = true;
            String::new()
        });
        assert!(!built, "detail must not be built while off");
        assert_eq!(
            COUNTS[Counter::Commands as usize].load(Ordering::Relaxed),
            before
        );
        assert!(current_path().is_none());
        assert!(!ui_sample_due());
    }

    #[test]
    fn start_writes_header_events_and_samples_then_stops() {
        let _g = serial();
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("diagnostics");
        let path = start(&dir, vec!["version: test".into()]).expect("start");
        assert!(enabled());
        assert_eq!(
            start(&dir, Vec::new()).expect("again"),
            path,
            "second start is a no-op"
        );
        event("cmd", || "media.toggle".into());
        count(Counter::Frames);
        count(Counter::Frames);
        send_ui_sample(UiSample {
            textures: 7,
            texture_reuse: true,
            videos: vec![(1920, 1080)],
            ..UiSample::default()
        });
        std::thread::sleep(Duration::from_millis(1300));
        stop();
        assert!(!enabled());
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("version: test"), "{text}");
        assert!(text.contains("os: "), "{text}");
        assert!(text.contains("cmd: media.toggle"), "{text}");
        assert!(text.contains(" frames=2"), "{text}");
        assert!(text.contains(" tex=7"), "{text}");
        assert!(text.contains(" video=1920x1080"), "{text}");
        assert!(text.trim_end().ends_with("log stopped"), "{text}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode(&dir), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
    }

    #[test]
    fn command_parameters_never_reach_the_log() {
        let _g = serial();
        let root = tempfile::tempdir().expect("tempdir");
        let path = start(&root.path().join("diagnostics"), Vec::new()).expect("start");
        let mut s = crate::Session::new().expect("session");
        let secret = root.path().join("SecretHolidayPhotos");
        let secret_text = secret.to_string_lossy().into_owned();
        let _ = s.dispatch(
            "prefs.set",
            &serde_json::json!({"key": "thumb_cache_dir", "value": secret_text}),
        );
        let _ = s.dispatch(
            "remote.connect.smb",
            &serde_json::json!({"host": "fileserver.example", "share": "private",
                "user": "alice", "password": "hunter2"}),
        );
        let _ = s.dispatch(
            "no.such.command.SecretHolidayPhotos",
            &serde_json::Value::Null,
        );
        stop();
        let text = std::fs::read_to_string(&path).expect("read");
        assert!(text.contains("cmd: prefs.set"), "{text}");
        for leaked in [
            "SecretHolidayPhotos",
            "fileserver",
            "alice",
            "hunter2",
            "private",
        ] {
            assert!(
                !text.contains(leaked),
                "{leaked} leaked into the log:\n{text}"
            );
        }
    }

    #[test]
    fn repeated_and_excess_events_are_bounded() {
        let mut state = WriterState::default();
        let mut out = Out {
            file: Vec::new(),
            written: 0,
            full: false,
            start: Instant::now(),
        };
        for _ in 0..50 {
            state.event(&mut out, "cmd: same".into());
        }
        for i in 0..1000 {
            state.event(&mut out, format!("tex: {i}"));
        }
        state.sample(&mut out, None);
        let text = String::from_utf8(out.file).expect("utf8");
        assert_eq!(text.matches("cmd: same").count(), 1);
        assert!(text.contains("repeated 49 times"), "{text}");
        let written = text.matches("tex: ").count() as u32;
        assert_eq!(
            written,
            MAX_EVENTS_PER_SEC - 1,
            "one slot went to the first event"
        );
        assert!(text.contains("events_suppressed="), "{text}");
    }

    #[test]
    fn the_file_stops_at_the_size_cap() {
        let mut out = Out {
            file: Vec::new(),
            written: MAX_LOG_BYTES - 100,
            full: false,
            start: Instant::now(),
        };
        out.line(&"x".repeat(200));
        out.line("after");
        let text = String::from_utf8(out.file).expect("utf8");
        assert!(text.contains("log size limit reached"));
        assert!(!text.contains("after"));
        assert!(out.full);
    }

    #[test]
    fn only_the_graphics_stack_is_accepted() {
        use log::Level::{Debug, Info, Trace, Warn};
        assert!(accepted("egui_glow::painter", Debug));
        assert!(!accepted("egui_glow::painter", Trace));
        assert!(accepted("winit::platform", Warn));
        assert!(!accepted("winit::platform", Info));
        for target in [
            "russh::client",
            "smb",
            "reqwest::connect",
            "rustls",
            "fm_engine",
        ] {
            assert!(!accepted(target, log::Level::Error), "{target}");
        }
    }

    #[test]
    fn pruning_keeps_the_newest_logs_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        for i in 0..25 {
            std::fs::write(
                dir.path().join(format!("diagnostics-2026010{i:02}.log")),
                "x",
            )
            .expect("write");
        }
        std::fs::write(dir.path().join("keep.txt"), "x").expect("write");
        prune(dir.path(), 19);
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 20);
        assert!(names.contains(&"keep.txt".to_string()));
        assert!(names.contains(&"diagnostics-202601024.log".to_string()));
        assert!(!names.contains(&"diagnostics-202601000.log".to_string()));
    }

    #[test]
    fn long_lines_are_cut_on_a_character_boundary() {
        let mut s = "é".repeat(400);
        truncate(&mut s, 101);
        assert!(s.len() <= 104);
        assert!(s.ends_with('…'));
    }
}
