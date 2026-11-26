//! RAR through an installed tool (design D9): 7-Zip (`7zz`, `7z`, `7za`, or `7z.exe` in the
//! standard Windows folder) or `unrar`, or a tool set in Preferences. Listing and member
//! reads run the tool with an argument array (never a shell), the archive and member as
//! single arguments after `--`, stdin closed, a timeout on listings and caps on what is
//! read back. Member names are matched literally (`-spd` for 7-Zip; unrar has no such
//! switch, so names with wildcard characters are refused). Remote RAR files are copied to
//! a private temporary folder first (at most [`crate::limits::spool_cap`] bytes, all volumes).

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{RecvTimeoutError, sync_channel};
use std::sync::{Arc, RwLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use fm_vfs::FmError;

use super::blocks::{Blocks, Rule};
use super::{Declared, FormatId, Info, Passwords, Progress, Reader};
use crate::Secret;
use crate::index::{Index, LiveIndex, Meta, NodeId, NodeKind};
use crate::source::Source;

/// Longest a listing of a small archive may take before the tool is stopped.
pub const LIST_TIMEOUT: Duration = Duration::from_secs(120);

/// The listing time allowed for a set of `bytes`: the tool reads every volume, so big sets
/// get five more seconds per GiB, up to an hour. Leaving the folder still stops it at once.
pub fn list_timeout(bytes: u64) -> Duration {
    let extra = (bytes >> 30).saturating_mul(5).min(3600);
    LIST_TIMEOUT
        .saturating_add(Duration::from_secs(extra))
        .min(Duration::from_secs(3600))
}
/// Most listing output read from the tool.
pub const MAX_LISTING: u64 = 64 << 20;
/// Most error output kept from the tool.
const MAX_STDERR: u64 = 64 << 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    SevenZip,
    Unrar,
}

/// A RAR-capable program and what kind it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tool {
    pub kind: ToolKind,
    pub path: PathBuf,
}

impl Tool {
    pub fn label(&self) -> &'static str {
        match self.kind {
            ToolKind::SevenZip => "7-Zip",
            ToolKind::Unrar => "unrar",
        }
    }
}

/// The tool chosen in Preferences, tried before the search.
static OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Use `path` (from Preferences) before searching; `None` searches only.
pub fn set_tool_override(path: Option<PathBuf>) {
    if let Ok(mut o) = OVERRIDE.write() {
        *o = path.filter(|p| !p.as_os_str().is_empty());
    }
}

/// The tool to use now: the Preferences choice, else `7zz`, `7z`, `7za` on `PATH`, else
/// (Windows) `%ProgramFiles%\7-Zip\7z.exe`, else `unrar` on `PATH`.
pub fn find_tool() -> Option<Tool> {
    let over = OVERRIDE.read().ok().and_then(|o| o.clone());
    let program_files = std::env::var_os("ProgramFiles").map(PathBuf::from);
    find_tool_in(
        over.as_deref(),
        std::env::var_os("PATH").as_deref(),
        program_files.as_deref(),
    )
}

fn kind_of(path: &Path) -> ToolKind {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if stem.contains("unrar") {
        ToolKind::Unrar
    } else {
        ToolKind::SevenZip
    }
}

/// [`find_tool`] with its inputs given (tests).
pub fn find_tool_in(
    over: Option<&Path>,
    path_var: Option<&OsStr>,
    program_files: Option<&Path>,
) -> Option<Tool> {
    if let Some(p) = over.filter(|p| p.is_file()) {
        return Some(Tool {
            kind: kind_of(p),
            path: p.to_path_buf(),
        });
    }
    let dirs: Vec<PathBuf> = path_var
        .map(|v| std::env::split_paths(v).collect())
        .unwrap_or_default();
    let exe = |name: &str| {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    };
    let on_path = |name: &str| dirs.iter().map(|d| d.join(exe(name))).find(|p| p.is_file());
    for name in ["7zz", "7z", "7za"] {
        if let Some(path) = on_path(name) {
            return Some(Tool {
                kind: ToolKind::SevenZip,
                path,
            });
        }
    }
    if let Some(path) = program_files
        .map(|p| p.join("7-Zip").join("7z.exe"))
        .filter(|p| p.is_file())
    {
        return Some(Tool {
            kind: ToolKind::SevenZip,
            path,
        });
    }
    on_path("unrar").map(|path| Tool {
        kind: ToolKind::Unrar,
        path,
    })
}

/// What to install on this system.
pub fn install_hint() -> &'static str {
    if cfg!(windows) {
        "install 7-Zip from 7-zip.org"
    } else if cfg!(target_os = "macos") {
        "install 7-Zip (brew install sevenzip) or unrar"
    } else {
        "install 7-Zip (package 7zip or p7zip-full) or unrar"
    }
}

/// "RAR archives need 7-Zip or unrar": the pane's state when no tool is found.
pub fn needs_tool_error() -> FmError {
    FmError::backend(
        "archive",
        format!(
            "RAR archives need 7-Zip or unrar: {}, or choose the program in Preferences › Files",
            install_hint()
        ),
    )
}

fn password_arg(kind: ToolKind, password: Option<&Secret>) -> Option<OsString> {
    match (kind, password) {
        (_, Some(pw)) => Some(OsString::from(format!("-p{}", pw.expose()))),
        // unrar would otherwise ask on the terminal; 7-Zip reads the closed stdin.
        (ToolKind::Unrar, None) => Some(OsString::from("-p-")),
        (ToolKind::SevenZip, None) => None,
    }
}

/// Arguments that list `archive`.
pub fn list_args(kind: ToolKind, password: Option<&Secret>, archive: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = match kind {
        ToolKind::SevenZip => ["l", "-slt", "-ba", "-sccUTF-8"]
            .map(OsString::from)
            .to_vec(),
        ToolKind::Unrar => vec![OsString::from("lt")],
    };
    args.extend(password_arg(kind, password));
    args.push(OsString::from("--"));
    args.push(archive.as_os_str().to_os_string());
    args
}

/// Arguments that write `member` of `archive` to stdout, matched literally.
pub fn read_args(
    kind: ToolKind,
    password: Option<&Secret>,
    archive: &Path,
    member: &str,
) -> Result<Vec<OsString>, FmError> {
    let mut args: Vec<OsString> = match kind {
        ToolKind::SevenZip => ["e", "-so", "-spd", "-bd", "-sccUTF-8"]
            .map(OsString::from)
            .to_vec(),
        ToolKind::Unrar => {
            if member.contains(['*', '?']) {
                return Err(FmError::Unsupported(
                    "unrar cannot read names with * or ?; install 7-Zip to read this file",
                ));
            }
            ["p", "-inul"].map(OsString::from).to_vec()
        }
    };
    args.extend(password_arg(kind, password));
    args.push(OsString::from("--"));
    args.push(archive.as_os_str().to_os_string());
    args.push(OsString::from(member));
    Ok(args)
}

fn command(tool: &Tool, args: &[OsString]) -> Command {
    let mut cmd = Command::new(&tool.path);
    cmd.args(args)
        // English messages, so password errors are recognised.
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        // 7z.exe and unrar.exe are console programs: without this the GUI flashes a
        // console window for every listing and member read.
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn spawn(cmd: &mut Command, tool: &Tool) -> Result<Child, FmError> {
    cmd.spawn().map_err(|e| {
        FmError::backend(
            "archive",
            format!(
                "could not start {} ({}): {e}",
                tool.label(),
                tool.path.display()
            ),
        )
    })
}

fn read_capped(r: impl Read + Send + 'static, cap: u64) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = r.take(cap + 1).read_to_end(&mut out);
        out
    })
}

/// What a finished tool run printed.
#[derive(Debug)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

/// Run `args` to the end, stopping it at `timeout`, on `cancelled`, or when stdout passes
/// `cap` bytes.
pub fn run_capture(
    tool: &Tool,
    args: &[OsString],
    timeout: Duration,
    cap: u64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Captured, FmError> {
    run_stream(tool, args, timeout, cap, cancelled, &mut |_| {})
}

/// [`run_capture`] that also hands every piece of stdout to `on_chunk` as it arrives.
pub fn run_stream(
    tool: &Tool,
    args: &[OsString],
    timeout: Duration,
    cap: u64,
    cancelled: &dyn Fn() -> bool,
    on_chunk: &mut dyn FnMut(&[u8]),
) -> Result<Captured, FmError> {
    let mut child = spawn(&mut command(tool, args), tool)?;
    let err = child.stderr.take().map(|e| read_capped(e, MAX_STDERR));
    let (tx, rx) = sync_channel::<Vec<u8>>(16);
    if let Some(mut out) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 << 10];
            while let Ok(n) = out.read(&mut buf) {
                let Some(piece) = buf.get(..n).filter(|p| !p.is_empty()) else {
                    break;
                };
                if tx.send(piece.to_vec()).is_err() {
                    break;
                }
            }
        });
    } else {
        drop(tx);
    }
    let start = Instant::now();
    let stop = |child: &mut Child, error: FmError| {
        let _ = child.kill();
        let _ = child.wait();
        error
    };
    let timed_out = || {
        FmError::backend(
            "archive",
            format!("{} took longer than {} s", tool.label(), timeout.as_secs()),
        )
    };
    let mut stdout = Vec::new();
    let mut open = true;
    let status = loop {
        if open {
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(chunk) => {
                    if stdout.len() as u64 + chunk.len() as u64 > cap {
                        return Err(stop(
                            &mut child,
                            FmError::backend("archive", "the archive listing is too large"),
                        ));
                    }
                    on_chunk(&chunk);
                    stdout.extend_from_slice(&chunk);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => open = false,
            }
        } else {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(FmError::Io(e)),
            }
        }
        if cancelled() {
            return Err(stop(&mut child, FmError::Cancelled));
        }
        if start.elapsed() >= timeout {
            return Err(stop(&mut child, timed_out()));
        }
    };
    let stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    Ok(Captured {
        status,
        stdout,
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

/// One entry of a tool's listing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listed {
    pub path: String,
    pub dir: bool,
    pub size: Option<u64>,
    pub packed: Option<u64>,
    pub modified: Option<SystemTime>,
    pub encrypted: bool,
    pub link: Option<String>,
}

/// `2024-01-02 03:04:05` (fractions after `.` or `,` ignored), as UTC.
fn parse_time(s: &str) -> Option<SystemTime> {
    let (date, time) = s.trim().split_once(' ')?;
    let mut d = date.split('-');
    let (y, mo, da) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    let time = time.split(['.', ',']).next()?;
    let mut t = time.split(':');
    let (h, mi, se) = (
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
        t.next()?.parse().ok()?,
    );
    crate::time::from_civil(y, mo, da, h, mi, se)
}

/// `7z l -slt -ba` output: blocks of `Key = Value` lines separated by blank lines.
pub fn parse_7z_slt(text: &str) -> Vec<Listed> {
    let mut out = Vec::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let mut e = Listed::default();
        let mut has_path = false;
        let mut archive_block = false;
        for line in block.lines() {
            let Some((k, v)) = line
                .split_once(" = ")
                .or_else(|| line.strip_suffix(" =").map(|k| (k, "")))
            else {
                continue;
            };
            match k.trim() {
                "Path" => {
                    e.path = v.to_string();
                    has_path = true;
                }
                "Type" | "Physical Size" | "Headers Size" => archive_block = true,
                "Folder" => e.dir |= v == "+",
                "Attributes" => e.dir |= v.starts_with('D'),
                "Size" => e.size = v.parse().ok(),
                "Packed Size" => e.packed = v.parse().ok(),
                "Modified" => e.modified = parse_time(v),
                "Encrypted" => e.encrypted = v == "+",
                "Symbolic Link" | "Link" if !v.is_empty() => e.link = Some(v.to_string()),
                _ => {}
            }
        }
        if has_path && !archive_block && !e.path.is_empty() {
            out.push(e);
        }
    }
    out
}

/// `unrar lt` output: a banner, then blocks starting with `Name: …` and `Key: Value` lines.
pub fn parse_unrar_lt(text: &str) -> Vec<Listed> {
    let mut out: Vec<Listed> = Vec::new();
    let mut cur: Option<Listed> = None;
    for line in text.replace("\r\n", "\n").lines() {
        let Some((k, v)) = line.split_once(": ") else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim_end());
        if k == "Name" {
            out.extend(cur.take());
            let (encrypted, name) = match v.strip_prefix('*') {
                Some(rest) => (true, rest),
                None => (false, v),
            };
            cur = Some(Listed {
                path: name.to_string(),
                encrypted,
                ..Listed::default()
            });
            continue;
        }
        let Some(e) = cur.as_mut() else {
            continue;
        };
        match k {
            "Type" => {
                let t = v.to_ascii_lowercase();
                e.dir |= t.starts_with("directory");
                if t.contains("symbolic link") || t.contains("junction") {
                    e.link = Some(String::new());
                }
            }
            "Target" => e.link = Some(v.to_string()),
            "Size" => e.size = v.parse().ok(),
            "Packed size" => e.packed = v.parse().ok(),
            "mtime" => e.modified = parse_time(v),
            "Flags" => e.encrypted |= v.to_ascii_lowercase().contains("encrypted"),
            _ => {}
        }
    }
    out.extend(cur);
    out
}

fn mentions_password(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "wrong password",
        "incorrect password",
        "password is incorrect",
        "enter password",
        "can not open encrypted archive",
        "cannot open encrypted archive",
        "encrypted headers",
    ]
    .iter()
    .any(|m| t.contains(m))
}

/// A failed tool run as an error: password problems become the password errors (so the
/// app asks), anything else names the tool and its first line of complaint.
fn classify(
    tool: &Tool,
    status: ExitStatus,
    stderr: &str,
    stdout: &[u8],
    name: &str,
    tried: bool,
    passwords: Option<&Passwords>,
) -> FmError {
    let tail = String::from_utf8_lossy(
        stdout
            .get(stdout.len().saturating_sub(4096)..)
            .unwrap_or_default(),
    )
    .into_owned();
    // unrar exits 11 for a wrong password.
    let unrar_bad_pw = tool.kind == ToolKind::Unrar && status.code() == Some(11);
    if unrar_bad_pw || mentions_password(stderr) || mentions_password(&tail) {
        return match (tried, passwords) {
            (true, Some(p)) => p.failed(name),
            (true, None) => FmError::WrongPassword(name.to_string()),
            (false, _) => FmError::PasswordRequired(name.to_string()),
        };
    }
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("it stopped with an error");
    let line: String = line.chars().take(200).collect();
    FmError::backend(
        "archive",
        format!("{} could not read {name}: {line}", tool.label()),
    )
}

pub use crate::spool::SpoolDir;

/// Copy the volumes of a remote RAR set (`(name, source)` in order) into a private folder
/// under their own names; returns the folder and the first volume's path.
pub fn spool(
    volumes: &[(String, Arc<dyn Source>)],
    report: &dyn Fn(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<(SpoolDir, PathBuf), FmError> {
    let total: u64 = volumes
        .iter()
        .map(|(_, s)| s.len())
        .fold(0, u64::saturating_add);
    let cap = crate::limits::spool_cap();
    if total > cap {
        return Err(FmError::backend(
            "archive",
            format!(
                "remote RAR archives over {} GiB cannot be opened here; copy it to this \
                 computer first, or raise the limit in Preferences › Files",
                cap >> 30
            ),
        ));
    }
    let dir = SpoolDir::create("fm-rar")?;
    let mut first = None;
    let mut copied = 0u64;
    for (name, src) in volumes {
        // Names come from a folder listing: a plain name, never a path.
        if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            return Err(FmError::InvalidParams(format!(
                "odd RAR volume name {name:?}"
            )));
        }
        let path = dir.path().join(name);
        let mut file = std::fs::File::create(&path).map_err(FmError::Io)?;
        let mut buf = vec![0u8; 1 << 20];
        let mut at = 0u64;
        while at < src.len() {
            if cancelled() {
                return Err(FmError::Cancelled);
            }
            let n = src.read_at(at, &mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(buf.get(..n).unwrap_or_default())
                .map_err(FmError::Io)?;
            at += n as u64;
            copied = copied.saturating_add(n as u64);
            report(copied, total);
        }
        first.get_or_insert(path);
    }
    let first = first.ok_or_else(|| FmError::NotFound("RAR volume".into()))?;
    Ok((dir, first))
}

/// A RAR archive read through the tool.
pub struct RarReader {
    tool: Tool,
    path: PathBuf,
    name: String,
    index: Index,
    /// The tool's own name for each member (`Meta::locator` indexes it).
    raw: Vec<String>,
    info: Info,
    header_password: Option<Secret>,
    _spool: Option<SpoolDir>,
}

/// The entries a listing run has produced, in the order of the tool's own numbering.
#[derive(Default)]
struct Listing {
    raw: Vec<String>,
    any_encrypted: bool,
    error: Option<FmError>,
}

impl Listing {
    /// Run the tool, adding each entry to `live` as soon as the tool has printed it.
    fn run(
        &mut self,
        tool: &Tool,
        args: &[OsString],
        timeout: Duration,
        cancelled: &dyn Fn() -> bool,
        live: &LiveIndex,
    ) -> Result<Captured, FmError> {
        *self = Self::default();
        live.clear();
        let rule = match tool.kind {
            ToolKind::SevenZip => Rule::BlankLine,
            ToolKind::Unrar => Rule::LineStart("Name:"),
        };
        let parse = match tool.kind {
            ToolKind::SevenZip => parse_7z_slt,
            ToolKind::Unrar => parse_unrar_lt,
        };
        let mut blocks = Blocks::new(rule);
        let mut take = |block: &str| {
            for e in parse(block) {
                self.add(live, e);
            }
        };
        let result = run_stream(tool, args, timeout, MAX_LISTING, cancelled, &mut |chunk| {
            blocks.feed(chunk, &mut take);
        });
        if result.is_ok() {
            blocks.finish(&mut take);
        }
        match (result, self.error.take()) {
            (Ok(out), None) => {
                if !out.status.success() {
                    live.clear();
                }
                Ok(out)
            }
            (_, Some(e)) => {
                live.clear();
                Err(e)
            }
            (Err(e), None) => {
                live.clear();
                Err(e)
            }
        }
    }

    fn add(&mut self, live: &LiveIndex, e: Listed) {
        if self.error.is_some() {
            return;
        }
        self.any_encrypted |= e.encrypted;
        let kind = if e.dir {
            NodeKind::Dir
        } else if let Some(target) = e.link.clone() {
            NodeKind::Symlink(target)
        } else {
            NodeKind::File
        };
        let meta = Meta {
            size: e.size,
            packed: e.packed,
            modified: e.modified,
            encrypted: e.encrypted,
            locator: self.raw.len() as u64,
            ..Meta::default()
        };
        match live.add(&e.path, kind, meta) {
            Ok(_) => self.raw.push(e.path),
            Err(err) => self.error = Some(err),
        }
    }
}

impl RarReader {
    /// List `path` (a local RAR, the first volume of a set of `bytes` in all) with `tool`.
    pub fn open(
        tool: Tool,
        path: PathBuf,
        name: &str,
        passwords: &Passwords,
        progress: &Progress<'_>,
        spool: Option<SpoolDir>,
        bytes: u64,
    ) -> Result<Self, FmError> {
        let timeout = list_timeout(bytes);
        Self::open_with(tool, path, name, passwords, progress, spool, timeout)
    }

    fn open_with(
        tool: Tool,
        path: PathBuf,
        name: &str,
        passwords: &Passwords,
        progress: &Progress<'_>,
        spool: Option<SpoolDir>,
        timeout: Duration,
    ) -> Result<Self, FmError> {
        let own = LiveIndex::new();
        let live = progress.live.unwrap_or(&own);
        let mut listing = Listing::default();
        let mut list = |pw: Option<&Secret>| {
            listing.run(
                &tool,
                &list_args(tool.kind, pw, &path),
                timeout,
                progress.cancelled,
                live,
            )
        };
        let mut header_password = None;
        let out = list(None)?;
        if !out.status.success() {
            let err = classify(
                &tool,
                out.status,
                &out.stderr,
                &out.stdout,
                name,
                false,
                None,
            );
            if !err.wants_password() {
                return Err(err);
            }
            // Names are hidden: try the known passwords, newest first.
            if passwords.gave_up() {
                return Err(super::gave_up_error(name));
            }
            let known = passwords.list();
            if known.is_empty() {
                return Err(err);
            }
            let mut found = None;
            for pw in known {
                let attempt = list(Some(&pw))?;
                if attempt.status.success() {
                    passwords.fitted(&pw);
                    found = Some(pw);
                    break;
                }
            }
            let Some(pw) = found else {
                return Err(passwords.failed(name));
            };
            header_password = Some(pw);
        }
        let Listing {
            raw, any_encrypted, ..
        } = listing;
        let index = live.take();
        let info = Info {
            methods: vec![format!("RAR (through {})", tool.label())],
            encrypted: any_encrypted || header_password.is_some(),
            names_encrypted: header_password.is_some(),
        };
        Ok(Self {
            tool,
            path,
            name: name.to_string(),
            index,
            raw,
            info,
            header_password,
            _spool: spool,
        })
    }
}

/// A member streamed from the tool's stdout. The end of the stream checks how the tool
/// exited; dropping it early stops the tool. A wrong password is only known at the end,
/// so it is reported but not counted toward the three-strikes limit (listings count).
struct ToolRead {
    child: Child,
    stdout: ChildStdout,
    stderr: Option<JoinHandle<Vec<u8>>>,
    tool: Tool,
    name: String,
    tried: bool,
    done: bool,
}

impl Read for ToolRead {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done {
            return Ok(0);
        }
        let n = self.stdout.read(buf)?;
        if n > 0 {
            return Ok(n);
        }
        self.done = true;
        let status = self.child.wait()?;
        if status.success() {
            return Ok(0);
        }
        let stderr = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let err = classify(
            &self.tool,
            status,
            &stderr,
            &[],
            &self.name,
            self.tried,
            None,
        );
        Err(io::Error::other(err.to_string()))
    }
}

impl Drop for ToolRead {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

impl Reader for RarReader {
    fn format(&self) -> FormatId {
        FormatId::Rar
    }

    fn index(&self) -> &Index {
        &self.index
    }

    fn info(&self) -> Info {
        self.info.clone()
    }

    fn open(&self, id: NodeId, passwords: &Passwords) -> Result<Box<dyn Read + Send>, FmError> {
        let node = self
            .index
            .node(id)
            .filter(|n| n.kind == NodeKind::File)
            .ok_or_else(|| FmError::NotFound(self.name.clone()))?;
        let member = usize::try_from(node.meta.locator)
            .ok()
            .and_then(|i| self.raw.get(i))
            .ok_or_else(|| FmError::NotFound(self.name.clone()))?;
        let password = if node.meta.encrypted {
            if passwords.gave_up() {
                return Err(super::gave_up_error(&self.name));
            }
            // The tool can only say "wrong" at the end of the stream: use the newest
            // password given (or the one that opened the names).
            let pw = passwords
                .list()
                .into_iter()
                .next()
                .or_else(|| self.header_password.clone());
            Some(pw.ok_or_else(|| FmError::PasswordRequired(self.name.clone()))?)
        } else {
            self.header_password.clone()
        };
        let args = read_args(self.tool.kind, password.as_ref(), &self.path, member)?;
        let mut child = spawn(&mut command(&self.tool, &args), &self.tool)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| FmError::backend("archive", "the tool gave no output"))?;
        let stderr = child.stderr.take().map(|e| read_capped(e, MAX_STDERR));
        let reader = ToolRead {
            child,
            stdout,
            stderr,
            tool: self.tool.clone(),
            name: self.name.clone(),
            tried: password.is_some(),
            done: false,
        };
        let size = node.meta.size.unwrap_or(u64::MAX);
        Ok(Box::new(Declared::new(reader, size)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLT: &str = "Path = docs\nFolder = +\nSize = 0\nPacked Size = 0\nModified = 2024-01-02 03:04:05\nAttributes = D\nEncrypted = -\n\nPath = docs/a.txt\nFolder = -\nSize = 5\nPacked Size = 17\nModified = 2024-01-02 03:04:05.1234567\nAttributes = A\nEncrypted = +\nMethod = m3:24\n\nPath = ../../evil\nFolder = -\nSize = 4\nPacked Size = 4\nModified = 2024-01-02 03:04:05\nEncrypted = -\n\nPath = link\nFolder = -\nSize = 6\nAttributes = A\nSymbolic Link = docs/a.txt\n";

    const UNRAR: &str = "\nUNRAR 7.00 freeware      Copyright (c) 1993-2024 Alexander Roshal\n\nArchive: test.rar\nDetails: RAR 5\n\n        Name: docs/a.txt\n        Type: File\n        Size: 5\n Packed size: 17\n       Ratio: 340%\n       mtime: 2024-01-02 03:04:05,000000000\n  Attributes: -rw-r--r--\n       CRC32: 12345678\n     Host OS: Unix\n Compression: RAR 5.0(v50) -m3 -md=128K\n       Flags: encrypted\n\n        Name: docs\n        Type: Directory\n       mtime: 2024-01-02 03:04:05,000000000\n\n        Name: lnk\n        Type: Unix symbolic link\n      Target: docs/a.txt\n        Size: 10\n";

    #[test]
    fn seven_zip_listings_parse() {
        let e = parse_7z_slt(SLT);
        assert_eq!(e.len(), 4);
        assert!(e[0].dir && e[0].path == "docs");
        assert_eq!(
            (e[1].size, e[1].packed, e[1].encrypted),
            (Some(5), Some(17), true)
        );
        assert_eq!(e[1].modified, crate::time::from_civil(2024, 1, 2, 3, 4, 5));
        assert_eq!(e[3].link.as_deref(), Some("docs/a.txt"));
        // An archive-level block (without -ba, or from a future version) is skipped.
        let with_header = format!("Path = test.rar\nType = Rar5\nPhysical Size = 99\n\n{SLT}");
        assert_eq!(parse_7z_slt(&with_header).len(), 4);
        assert!(parse_7z_slt("garbage\n\n=\n = \n").is_empty());
    }

    #[test]
    fn unrar_listings_parse() {
        let e = parse_unrar_lt(UNRAR);
        assert_eq!(e.len(), 3);
        assert_eq!(
            (e[0].path.as_str(), e[0].size, e[0].encrypted),
            ("docs/a.txt", Some(5), true)
        );
        assert!(e[1].dir);
        assert_eq!(e[2].link.as_deref(), Some("docs/a.txt"));
        let starred =
            parse_unrar_lt("        Name: *secret.txt\n        Type: File\n        Size: 3\n");
        assert!(starred[0].encrypted && starred[0].path == "secret.txt");
    }

    #[test]
    fn arguments_are_arrays_with_the_archive_and_member_after_double_dash() {
        let archive = Path::new("/tmp/a -rf.rar");
        let pw = Secret::new("p w");
        let args = read_args(ToolKind::SevenZip, Some(&pw), archive, "-x;rm *").expect("7z");
        let dd = args.iter().position(|a| a == "--").expect("--");
        assert_eq!(
            args.get(dd + 1).map(OsString::as_os_str),
            Some(archive.as_os_str())
        );
        assert_eq!(
            args.get(dd + 2)
                .map(|a| a.to_string_lossy().into_owned())
                .as_deref(),
            Some("-x;rm *")
        );
        assert!(args.iter().any(|a| a == "-spd"), "names matched literally");
        assert!(args.iter().any(|a| a == "-pp w"));
        assert!(
            read_args(ToolKind::Unrar, None, archive, "a*b").is_err(),
            "unrar wildcard"
        );
        let u = list_args(ToolKind::Unrar, None, archive);
        assert!(
            u.iter().any(|a| a == "-p-"),
            "unrar never asks on a terminal"
        );
        assert!(
            !list_args(ToolKind::SevenZip, None, archive)
                .iter()
                .any(|a| a.to_string_lossy().starts_with("-p"))
        );
    }

    #[test]
    fn big_sets_get_more_listing_time_up_to_an_hour() {
        assert_eq!(list_timeout(0), LIST_TIMEOUT);
        let set = 36 * (4u64 << 30);
        assert_eq!(
            list_timeout(set),
            LIST_TIMEOUT + Duration::from_secs(36 * 4 * 5)
        );
        assert_eq!(list_timeout(u64::MAX), Duration::from_secs(3600));
    }

    #[test]
    fn tool_search_order() {
        let dir = tempfile::tempdir().expect("tmp");
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).expect("bin");
        let exe = |n: &str| {
            if cfg!(windows) {
                format!("{n}.exe")
            } else {
                n.to_string()
            }
        };
        let path_var = std::env::join_paths([&bin]).expect("PATH");
        assert_eq!(find_tool_in(None, Some(&path_var), None), None);
        std::fs::write(bin.join(exe("unrar")), b"").expect("unrar");
        assert_eq!(
            find_tool_in(None, Some(&path_var), None).map(|t| t.kind),
            Some(ToolKind::Unrar)
        );
        std::fs::write(bin.join(exe("7za")), b"").expect("7za");
        let t = find_tool_in(None, Some(&path_var), None).expect("7za");
        assert_eq!(
            (
                t.kind,
                t.path.file_name().map(|n| n.to_string_lossy().into_owned())
            ),
            (ToolKind::SevenZip, Some(exe("7za")))
        );
        let mine = dir.path().join("my-unrar");
        std::fs::write(&mine, b"").expect("mine");
        assert_eq!(
            find_tool_in(Some(&mine), Some(&path_var), None).map(|t| t.kind),
            Some(ToolKind::Unrar),
            "Preferences first"
        );
        assert!(needs_tool_error().to_string().contains("7-Zip"));
    }

    /// A stand-in tool: lists `listing.txt`, prints a member, says "Wrong password" for
    /// `-pbad`, sleeps for `slow.rar`, lists `stream.rar` in two steps, and records its arguments.
    #[cfg(unix)]
    fn fake_tool(dir: &Path) -> Tool {
        use std::os::unix::fs::PermissionsExt as _;
        let script = dir.join("7z");
        std::fs::write(
            &script,
            r#"#!/bin/sh
d=$(dirname "$0")
printf '%s\n' "$@" > "$d/args.txt"
for a; do last=$a; case "$a" in -pbad) echo "ERROR: Wrong password : x" >&2; exit 2;; *slow.rar) sleep 5;; *stream.rar) s=1;; esac; done
case "$1" in
  l) if [ -n "$s" ]; then printf 'Path = one.txt\nSize = 1\n\n'; sleep 2; printf 'Path = two.txt\nSize = 2\n\n'; else cat "$d/listing.txt"; fi;;
  e) printf 'data';;
esac
"#,
        )
        .expect("script");
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        std::fs::write(dir.join("listing.txt"), SLT).expect("listing");
        Tool {
            kind: ToolKind::SevenZip,
            path: script,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_tool_lists_streams_and_reports_passwords() {
        let dir = tempfile::tempdir().expect("tmp");
        let tool = fake_tool(dir.path());
        let rar = dir.path().join("test.rar");
        std::fs::write(&rar, b"Rar!").expect("rar");
        let pw = Passwords::default();
        let r = RarReader::open(
            tool.clone(),
            rar,
            "test.rar",
            &pw,
            &Progress::none(),
            None,
            0,
        )
        .expect("listing");
        let a = r.index().lookup("docs/a.txt").expect("a");
        let evil = r.index().lookup("evil").expect("clamped inside the root");
        let mut s = String::new();
        r.open(evil, &pw)
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "data");
        let args = std::fs::read_to_string(dir.path().join("args.txt")).expect("args");
        assert!(
            args.lines().any(|l| l == "../../evil"),
            "the tool gets its own name: {args}"
        );
        // Encrypted member: no password asks; a wrong one says so at the end of the read.
        assert!(matches!(r.open(a, &pw), Err(FmError::PasswordRequired(_))));
        pw.add(Secret::new("bad"));
        let mut out = Vec::new();
        let err = r
            .open(a, &pw)
            .expect("spawn")
            .read_to_end(&mut out)
            .expect_err("wrong");
        assert!(
            err.to_string().starts_with("wrong password for test.rar"),
            "{err}"
        );
        // Entries can be used while the tool is still printing.
        let stream = dir.path().join("stream.rar");
        std::fs::write(&stream, b"Rar!").expect("stream");
        let live = LiveIndex::new();
        let progress = Progress {
            live: Some(&live),
            ..Progress::none()
        };
        let (tool2, pw2) = (tool.clone(), Passwords::default());
        let seen = std::thread::scope(|s| {
            let open =
                s.spawn(|| RarReader::open(tool2, stream, "stream.rar", &pw2, &progress, None, 0));
            let started = Instant::now();
            while live.is_empty() && started.elapsed() < Duration::from_secs(1) {
                std::thread::sleep(Duration::from_millis(20));
            }
            let seen = live.len();
            let done = open.join().expect("thread").expect("listing");
            (seen, done.index().len())
        });
        assert_eq!(seen, (1, 2), "one entry before the tool ended, both after");
        assert!(live.is_empty(), "the finished index moved to the reader");
        // The listing stops at its timeout.
        let slow = dir.path().join("slow.rar");
        std::fs::write(&slow, b"Rar!").expect("slow");
        let started = Instant::now();
        let err = RarReader::open_with(
            tool,
            slow,
            "slow.rar",
            &pw,
            &Progress::none(),
            None,
            Duration::from_millis(200),
        )
        .err()
        .expect("timeout");
        assert!(err.to_string().contains("took longer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(4));
    }
}
