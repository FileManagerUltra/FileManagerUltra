use std::io::{Read, Write};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use fm_vfs::FmError;
use portable_pty::{Child, MasterPty, PtySize, native_pty_system};

use crate::emulator::{Emulator, MAX_COLS, MAX_ROWS, MIN_SIZE, Snapshot};
use crate::quote::Dialect;
use crate::shell::Shell;

/// Largest single write accepted from the UI (keystrokes, pastes).
pub const MAX_INPUT: usize = 1024 * 1024;
/// Bytes parsed per [`TermSession::pump`] call, so output floods cannot stall a frame.
pub const PUMP_BUDGET: usize = 256 * 1024;
const CHUNK: usize = 16 * 1024;
const QUEUE_CHUNKS: usize = 64;

/// Whether a parent environment variable is handed to the child. App-internal and build
/// variables stay out so the shell does not see (or leak) them.
pub fn spawn_env_allowed(name: &str) -> bool {
    !(name.starts_with("FM_") || name.starts_with("CARGO_") || name == "RUST_BACKTRACE")
}

/// Result of one [`TermSession::pump`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pump {
    /// The screen or the exit status changed.
    pub changed: bool,
    /// More output is already queued: ask for another frame.
    pub backlog: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Running,
    Exited(i32),
}

fn io_err(e: impl std::fmt::Display) -> FmError {
    FmError::backend("term", e.to_string())
}

/// One shell on a pty. The parser lives on the caller's thread; reading and writing the
/// pty happen on helper threads, so nothing here blocks a frame.
pub struct TermSession {
    emulator: Emulator,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    output: Receiver<Vec<u8>>,
    input: Sender<Vec<u8>>,
    status: Status,
    reader: Option<JoinHandle<()>>,
    dialect: Dialect,
}

impl TermSession {
    pub fn scrollback_capacity(&self) -> usize {
        self.emulator.scrollback_capacity()
    }

    /// Start `shell` in `cwd` (or the shell's default directory).
    pub fn spawn(shell: Shell, cwd: Option<&str>, rows: u16, cols: u16) -> Result<Self, FmError> {
        Self::spawn_with_scrollback(shell, cwd, rows, cols, crate::SCROLLBACK)
    }

    pub fn spawn_with_scrollback(
        shell: Shell,
        cwd: Option<&str>,
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> Result<Self, FmError> {
        let (program, dialect) = shell.resolve()?;
        let rows = rows.clamp(MIN_SIZE, MAX_ROWS);
        let cols = cols.clamp(MIN_SIZE, MAX_COLS);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(io_err)?;

        let mut cmd = program.command();
        for (name, _) in std::env::vars_os() {
            if name.to_str().is_none_or(|n| !spawn_env_allowed(n)) {
                cmd.env_remove(name);
            }
        }
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }
        let child = pair.slave.spawn_command(cmd).map_err(io_err)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
        let mut writer = pair.master.take_writer().map_err(io_err)?;

        let (out_tx, output) = bounded::<Vec<u8>>(QUEUE_CHUNKS);
        let reader = std::thread::Builder::new()
            .name("fm-term-read".into())
            .spawn(move || {
                let mut buf = vec![0u8; CHUNK];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if out_tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(FmError::Io)?;

        let (input, in_rx) = bounded::<Vec<u8>>(QUEUE_CHUNKS);
        std::thread::Builder::new()
            .name("fm-term-write".into())
            .spawn(move || {
                while let Ok(bytes) = in_rx.recv() {
                    if writer
                        .write_all(&bytes)
                        .and_then(|()| writer.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(FmError::Io)?;

        Ok(Self {
            emulator: Emulator::with_scrollback(rows, cols, scrollback),
            master: pair.master,
            child,
            output,
            input,
            status: Status::Running,
            reader: Some(reader),
            dialect,
        })
    }

    /// Queue bytes for the shell. Typing snaps the view back to the live screen.
    pub fn write(&mut self, bytes: &[u8]) -> Result<(), FmError> {
        if bytes.len() > MAX_INPUT {
            return Err(FmError::InvalidParams(format!(
                "input larger than {MAX_INPUT} bytes"
            )));
        }
        if bytes.is_empty() || self.status != Status::Running {
            return Ok(());
        }
        self.emulator.scroll_to_bottom();
        match self.input.try_send(bytes.to_vec()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(FmError::backend("term", "input queue full")),
            Err(TrySendError::Disconnected(_)) => Ok(()),
        }
    }

    /// Feed pending output to the parser (bounded by [`PUMP_BUDGET`]) and notice exit.
    pub fn pump(&mut self) -> Pump {
        let mut used = 0;
        let was = self.status;
        while used < PUMP_BUDGET {
            match self.output.try_recv() {
                Ok(chunk) => {
                    used += chunk.len();
                    self.emulator.feed(&chunk);
                }
                Err(_) => break,
            }
        }
        let replies = self.emulator.take_replies();
        if !replies.is_empty() {
            // Best effort: a full queue only loses an answer, never blocks the frame.
            let _ = self.input.try_send(replies);
        }
        let backlog = !self.output.is_empty();
        if !backlog
            && self.status == Status::Running
            && let Ok(Some(code)) = self.child.try_wait().map(|s| s.map(|s| s.exit_code()))
        {
            self.status = Status::Exited(i32::try_from(code).unwrap_or(i32::MAX));
        }
        Pump {
            changed: used > 0 || was != self.status,
            backlog,
        }
    }

    /// Text the program asked to copy to the clipboard (OSC 52) since the last call.
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.emulator.take_clipboard()
    }

    /// The command language of the running shell, for quoting paths sent to it.
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn status(&self) -> Status {
        self.status
    }

    pub fn mouse(&mut self, event: crate::MouseEvent) -> Result<(), FmError> {
        let bytes = self.emulator.mouse_report(event)?;
        self.write(&bytes)
    }

    /// Whether something other than the shell owns the terminal (a running command).
    /// Always `false` where the platform cannot tell.
    pub fn has_foreground_job(&self) -> bool {
        if self.status != Status::Running {
            return false;
        }
        #[cfg(unix)]
        {
            if let (Some(fg), Some(shell)) =
                (self.master.process_group_leader(), self.child.process_id())
            {
                return u32::try_from(fg).is_ok_and(|fg| fg != shell);
            }
        }
        false
    }

    pub fn process_id(&self) -> Option<u32> {
        self.child.process_id()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        let rows = rows.clamp(MIN_SIZE, MAX_ROWS);
        let cols = cols.clamp(MIN_SIZE, MAX_COLS);
        if self.emulator.size() == (rows, cols) {
            return;
        }
        self.emulator.resize(rows, cols);
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
    }

    pub fn scroll(&mut self, lines: i32) {
        self.emulator.scroll(lines);
    }

    pub fn text_between(&mut self, a: (usize, u16), b: (usize, u16)) -> String {
        self.emulator.text_between(a, b)
    }

    pub fn clear_scrollback(&mut self) {
        self.emulator.clear_scrollback();
    }

    pub fn snapshot(&self) -> Snapshot {
        self.emulator.snapshot()
    }
}

impl std::fmt::Debug for TermSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermSession")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Drop for TermSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.try_wait();
        // The reader ends when the pty closes; never join (it could block on a stuck child).
        drop(self.reader.take());
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn spawn() -> Option<TermSession> {
        TermSession::spawn(Shell::Default, Some("/tmp"), 24, 80).ok()
    }

    fn wait_for(s: &mut TermSession, needle: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            s.pump();
            let snap = s.snapshot();
            if snap
                .text_between((0, 0), (snap.rows - 1, snap.cols - 1))
                .contains(needle)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn echo_round_trips_through_the_pty() {
        let Some(mut s) = spawn() else { return };
        s.write(b"echo fm-$((20+22))-ok\r").expect("write");
        assert!(wait_for(&mut s, "fm-42-ok"), "echo output never arrived");
    }

    #[test]
    fn starts_in_the_requested_directory() {
        let Some(mut s) = spawn() else { return };
        s.write(b"pwd\r").expect("write");
        assert!(wait_for(&mut s, "/tmp"));
    }

    #[test]
    fn resize_reaches_the_shell() {
        let Some(mut s) = spawn() else { return };
        s.resize(10, 33);
        s.write(b"stty size\r").expect("write");
        assert!(wait_for(&mut s, "10 33"));
        assert_eq!(s.snapshot().cols, 33);
    }

    #[test]
    fn exit_code_is_reported() {
        let Some(mut s) = spawn() else { return };
        s.write(b"exit 7\r").expect("write");
        let deadline = Instant::now() + Duration::from_secs(10);
        while s.status() == Status::Running && Instant::now() < deadline {
            s.pump();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(s.status(), Status::Exited(7));
        s.write(b"ignored").expect("writes after exit are ignored");
    }

    #[test]
    fn dropping_the_session_kills_the_child() {
        let Some(s) = spawn() else { return };
        let Some(pid) = s.process_id() else { return };
        let proc_dir = format!("/proc/{pid}");
        drop(s);
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::path::Path::new(&proc_dir).exists() && Instant::now() < deadline {
            // A killed, unreaped child lingers as a zombie; reading its state tells them apart.
            let state = std::fs::read_to_string(format!("{proc_dir}/stat")).unwrap_or_default();
            if state
                .split(')')
                .nth(1)
                .is_some_and(|r| r.trim_start().starts_with('Z'))
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!std::path::Path::new(&proc_dir).exists() || cfg!(not(target_os = "linux")));
    }

    #[test]
    fn output_flood_is_pumped_within_the_budget() {
        let Some(mut s) = spawn() else { return };
        s.write(b"yes flood | head -c 20000000\r").expect("write");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_backlog = false;
        while Instant::now() < deadline && !saw_backlog {
            saw_backlog = s.pump().backlog;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            saw_backlog,
            "a flood should leave a backlog beyond one frame's budget"
        );
    }

    #[test]
    fn oversized_input_is_rejected() {
        let Some(mut s) = spawn() else { return };
        assert!(s.write(&vec![b'x'; MAX_INPUT + 1]).is_err());
    }

    #[test]
    fn app_variables_are_not_passed_on() {
        assert!(!spawn_env_allowed("FM_WAYLAND"));
        assert!(!spawn_env_allowed("CARGO_TARGET_DIR"));
        assert!(spawn_env_allowed("PATH"));
        assert!(spawn_env_allowed("HOME"));
    }
}
