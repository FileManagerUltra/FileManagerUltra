//! Load and save jobs of the text editor (capability `text-editor`). They run on the
//! file's job lane, talk to the backend only through `Vfs`, and report back through the
//! editor inbox that `Session::poll` drains. Nothing here touches session state.

use std::io::Read;
use std::sync::{Arc, Mutex};

use fm_editor::{Buffer, DecodeError, Decoder, Encoding, SNIFF_BYTES, encode_into, sniff};
use fm_layout::PaneId;
use fm_vfs::{Entry, EntryKind, FmError, Vfs, VfsUri};

use crate::editor_doc::{Baseline, Conflict, MAX_EDIT_BYTES};

/// Bytes read per call while loading.
const READ_CHUNK: usize = 256 * 1024;
/// Decoded text is handed to the UI thread in pieces of about this size; the first
/// piece goes out at once so the first screen shows before the rest has arrived.
const TEXT_BATCH: usize = 1024 * 1024;
/// How many symlinks a load or save follows before giving up.
const MAX_LINK_HOPS: usize = 8;

/// Why a file is shown without an editable buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocked {
    Binary,
    TooLarge(u64),
    NotAFile,
}

impl Blocked {
    pub fn message(&self) -> String {
        match self {
            Self::Binary => {
                "This looks like a binary file, so it is not opened for editing.".to_string()
            }
            Self::TooLarge(size) => format!(
                "This file is {} MiB; files over {} MiB are not opened for editing.",
                size >> 20,
                MAX_EDIT_BYTES >> 20
            ),
            Self::NotAFile => "Only files can be edited.".to_string(),
        }
    }
}

/// Why a save did not write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveFail {
    Conflict(Conflict),
    Error(String),
    Cancelled,
}

#[derive(Debug)]
pub(crate) enum EdMsg {
    /// The file was found and decoding starts.
    Opened {
        pane: PaneId,
        token: u64,
        real: VfsUri,
        encoding: Encoding,
        baseline: Baseline,
    },
    Blocked {
        pane: PaneId,
        token: u64,
        why: Blocked,
    },
    /// Invalid in the sniffed encoding after all: start again as Latin-1.
    Restart {
        pane: PaneId,
        token: u64,
        encoding: Encoding,
    },
    Text {
        pane: PaneId,
        token: u64,
        text: String,
        loaded: u64,
    },
    Loaded {
        pane: PaneId,
        token: u64,
        result: Result<(), String>,
    },
    Saved {
        pane: PaneId,
        token: u64,
        /// Buffer version that was written.
        version: u64,
        /// Where it was written (differs from the document's file after Save as).
        to: VfsUri,
        result: Result<Baseline, SaveFail>,
    },
    /// Matches counted on a worker (large files).
    Found {
        pane: PaneId,
        token: u64,
        rev: u64,
        version: u64,
        result: Result<(Vec<fm_editor::Match>, bool), String>,
    },
    /// A syntax parse finished on its worker.
    Parsed {
        pane: PaneId,
        token: u64,
        result: Result<(fm_editor::SyntaxTree, u64), fm_editor::SyntaxError>,
        /// Fold ranges of the new tree (worked out on the same worker).
        folds: Vec<(usize, usize)>,
    },
    /// The file on disk as it is now (`None`: it is gone).
    Checked {
        pane: PaneId,
        token: u64,
        result: Result<Option<Baseline>, String>,
    },
}

pub(crate) type Inbox = Arc<Mutex<Vec<EdMsg>>>;

pub(crate) fn push(inbox: &Inbox, msg: EdMsg) {
    if let Ok(mut g) = inbox.lock() {
        g.push(msg);
    }
}

/// `uri` with its symlinks followed, and the entry of the file it ends at. Editing a
/// link edits the file it points to (that is the text the user sees); the link stays.
pub(crate) fn resolve(vfs: &dyn Vfs, uri: &VfsUri) -> Result<(VfsUri, Entry), FmError> {
    let mut at = uri.clone();
    for _ in 0..=MAX_LINK_HOPS {
        let entry = vfs.stat(&at)?;
        let EntryKind::Symlink { target } = &entry.kind else {
            return Ok((at, entry));
        };
        at = link_target(&at, target)?;
    }
    Err(FmError::InvalidParams(
        "too many symbolic links to follow".into(),
    ))
}

fn link_target(link: &VfsUri, target: &str) -> Result<VfsUri, FmError> {
    let bad = || FmError::InvalidParams("the link target cannot be followed".into());
    let target = target.replace('\\', "/");
    let mut base = if target.starts_with('/') {
        link.with_path("/")?
    } else {
        link.parent().ok_or_else(bad)?
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => base = base.parent().ok_or_else(bad)?,
            name => base = base.join(name)?,
        }
    }
    Ok(base)
}

/// Streams `uri` into the inbox as decoded text.
pub(crate) fn load(
    vfs: &dyn Vfs,
    uri: &VfsUri,
    pane: PaneId,
    token: u64,
    inbox: &Inbox,
    cancelled: &dyn Fn() -> bool,
) {
    let result = load_inner(vfs, uri, pane, token, inbox, cancelled);
    let result = match result {
        Ok(()) => Ok(()),
        Err(FmError::Cancelled) => return,
        Err(e) => Err(e.to_string()),
    };
    push(
        inbox,
        EdMsg::Loaded {
            pane,
            token,
            result,
        },
    );
}

fn load_inner(
    vfs: &dyn Vfs,
    uri: &VfsUri,
    pane: PaneId,
    token: u64,
    inbox: &Inbox,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), FmError> {
    let (real, entry) = resolve(vfs, uri)?;
    let block = |why| {
        push(inbox, EdMsg::Blocked { pane, token, why });
        Ok(())
    };
    if entry.kind != EntryKind::File {
        return block(Blocked::NotAFile);
    }
    if let Some(size) = entry.size.filter(|s| *s > MAX_EDIT_BYTES) {
        return block(Blocked::TooLarge(size));
    }
    let mut reader = vfs.open_read(&real)?;
    let mut head = vec![0u8; SNIFF_BYTES];
    let got = fill(&mut *reader, &mut head)?;
    head.truncate(got);
    let detected = sniff(&head, got < SNIFF_BYTES);
    if detected.binary {
        return block(Blocked::Binary);
    }
    push(
        inbox,
        EdMsg::Opened {
            pane,
            token,
            real: real.clone(),
            encoding: detected.encoding,
            baseline: Baseline::of(&entry),
        },
    );
    let mut stream = Stream {
        pane,
        token,
        inbox,
        cancelled,
        loaded: 0,
        pending: String::new(),
        sent_first: false,
    };
    match stream.run(detected.encoding, &head, &mut *reader) {
        Err(Failure::Invalid) => {
            // Valid UTF-8 at the start but not further on: read it all again as Latin-1,
            // where every byte is a character.
            push(
                inbox,
                EdMsg::Restart {
                    pane,
                    token,
                    encoding: Encoding::Latin1,
                },
            );
            let mut reader = vfs.open_read(&real)?;
            stream.loaded = 0;
            stream.pending.clear();
            stream.sent_first = false;
            stream
                .run(Encoding::Latin1, &[], &mut *reader)
                .map_err(Failure::into_error)
        }
        other => other.map_err(Failure::into_error),
    }
}

/// Reads until `buf` is full or the data ends; returns how much was read.
fn fill(r: &mut dyn Read, buf: &mut [u8]) -> Result<usize, FmError> {
    let mut at = 0;
    while at < buf.len() {
        let Some(rest) = buf.get_mut(at..) else {
            break;
        };
        let n = r.read(rest)?;
        if n == 0 {
            break;
        }
        at += n;
    }
    Ok(at)
}

enum Failure {
    Invalid,
    Fm(FmError),
}

impl Failure {
    fn into_error(self) -> FmError {
        match self {
            Self::Invalid => FmError::InvalidParams(DecodeError::Invalid.to_string()),
            Self::Fm(e) => e,
        }
    }
}

impl From<FmError> for Failure {
    fn from(e: FmError) -> Self {
        Self::Fm(e)
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::Fm(e.into())
    }
}

struct Stream<'a> {
    pane: PaneId,
    token: u64,
    inbox: &'a Inbox,
    cancelled: &'a dyn Fn() -> bool,
    loaded: u64,
    pending: String,
    sent_first: bool,
}

impl Stream<'_> {
    fn run(&mut self, enc: Encoding, head: &[u8], r: &mut dyn Read) -> Result<(), Failure> {
        let mut decoder = Decoder::new(enc);
        self.take(&mut decoder, head)?;
        let mut buf = vec![0u8; READ_CHUNK];
        loop {
            if (self.cancelled)() {
                return Err(Failure::Fm(FmError::Cancelled));
            }
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            if self.loaded + n as u64 > MAX_EDIT_BYTES {
                return Err(Failure::Fm(FmError::InvalidParams(
                    "the file grew past the editing limit while it was read".into(),
                )));
            }
            self.take(&mut decoder, buf.get(..n).unwrap_or_default())?;
        }
        decoder.finish().map_err(|e| match e {
            DecodeError::Invalid => Failure::Invalid,
            DecodeError::Truncated if enc == Encoding::Utf8 => Failure::Invalid,
            DecodeError::Truncated => Failure::Fm(FmError::InvalidParams(e.to_string())),
        })?;
        self.flush();
        Ok(())
    }

    fn take(&mut self, decoder: &mut Decoder, bytes: &[u8]) -> Result<(), Failure> {
        if bytes.is_empty() {
            return Ok(());
        }
        let text = decoder.push(bytes).map_err(|e| match e {
            DecodeError::Invalid | DecodeError::Truncated => Failure::Invalid,
        })?;
        self.loaded += bytes.len() as u64;
        self.pending.push_str(&text);
        if !self.sent_first || self.pending.len() >= TEXT_BATCH {
            self.flush();
        }
        Ok(())
    }

    fn flush(&mut self) {
        self.sent_first = true;
        push(
            self.inbox,
            EdMsg::Text {
                pane: self.pane,
                token: self.token,
                text: std::mem::take(&mut self.pending),
                loaded: self.loaded,
            },
        );
    }
}

/// What a save writes and how.
pub(crate) struct SaveJob {
    pub pane: PaneId,
    pub token: u64,
    pub to: VfsUri,
    pub buffer: Buffer,
    pub encoding: Encoding,
    /// Compare with the file on disk first (`None`: a new file, or the user chose to
    /// overwrite whatever is there).
    pub expect: Option<Baseline>,
    pub permissions: Option<u32>,
    /// Save as without overwrite: refuse when the target exists.
    pub refuse_existing: bool,
}

pub(crate) fn save(vfs: &dyn Vfs, job: SaveJob, inbox: &Inbox, cancelled: &dyn Fn() -> bool) {
    let version = job.buffer.version();
    let result = save_inner(vfs, &job, cancelled);
    push(
        inbox,
        EdMsg::Saved {
            pane: job.pane,
            token: job.token,
            version,
            to: job.to,
            result,
        },
    );
}

fn save_inner(
    vfs: &dyn Vfs,
    job: &SaveJob,
    cancelled: &dyn Fn() -> bool,
) -> Result<Baseline, SaveFail> {
    if job.refuse_existing {
        match vfs.stat(&job.to) {
            Ok(_) => {
                return Err(SaveFail::Error(format!(
                    "{} already exists; choose another name",
                    job.to.file_name().unwrap_or("the file")
                )));
            }
            Err(FmError::NotFound(_)) => {}
            Err(e) => return Err(SaveFail::Error(e.to_string())),
        }
    }
    if let Some(expect) = &job.expect {
        match vfs.stat(&job.to) {
            Ok(now) if expect.differs(&Baseline::of(&now)) => {
                return Err(SaveFail::Conflict(Conflict::Changed));
            }
            Ok(_) => {}
            Err(FmError::NotFound(_)) => return Err(SaveFail::Conflict(Conflict::Deleted)),
            Err(e) => return Err(SaveFail::Error(e.to_string())),
        }
    }
    let mut bytes = Vec::with_capacity(job.buffer.len_bytes() + 4);
    if job.encoding.writes_bom() {
        bytes.extend_from_slice(job.encoding.bom());
    }
    for (i, chunk) in job.buffer.chunks().enumerate() {
        if i % 256 == 0 && cancelled() {
            return Err(SaveFail::Cancelled);
        }
        encode_into(job.encoding, chunk, &mut bytes).map_err(|e| SaveFail::Error(e.to_string()))?;
    }
    match fm_ops::replace_contents(vfs, &job.to, &bytes, job.permissions, cancelled) {
        Ok(entry) => Ok(Baseline::of(&entry)),
        Err(FmError::Cancelled) => Err(SaveFail::Cancelled),
        Err(e) => Err(SaveFail::Error(e.to_string())),
    }
}

/// Looks at the file on disk without reading it.
pub(crate) fn check(vfs: &dyn Vfs, real: &VfsUri, pane: PaneId, token: u64, inbox: &Inbox) {
    let result = match vfs.stat(real) {
        Ok(entry) => Ok(Some(Baseline::of(&entry))),
        Err(FmError::NotFound(_)) => Ok(None),
        Err(e) => Err(e.to_string()),
    };
    push(
        inbox,
        EdMsg::Checked {
            pane,
            token,
            result,
        },
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;
    use fm_testkit::MemFs;

    fn uri(p: &str) -> VfsUri {
        VfsUri::parse(p).expect("uri")
    }

    fn drain(inbox: &Inbox) -> Vec<EdMsg> {
        std::mem::take(&mut *inbox.lock().expect("lock"))
    }

    fn text_of(msgs: &[EdMsg]) -> String {
        let mut out = String::new();
        for m in msgs {
            match m {
                EdMsg::Text { text, .. } => out.push_str(text),
                EdMsg::Restart { .. } => out.clear(),
                _ => {}
            }
        }
        out
    }

    #[test]
    fn loads_in_pieces_and_falls_back_to_latin1() {
        let mem = MemFs::new();
        let mut big = "abc\n".repeat(SNIFF_BYTES);
        big.push_str("caf");
        let mut bytes = big.clone().into_bytes();
        bytes.push(0xE9); // Latin-1 é: invalid UTF-8 past the sniffed head
        mem.put_file(&uri("/a.txt"), &bytes).expect("put");
        let inbox = Inbox::default();
        load(&mem, &uri("/a.txt"), 1, 7, &inbox, &|| false);
        let msgs = drain(&inbox);
        assert!(msgs.iter().any(|m| matches!(
            m,
            EdMsg::Restart {
                encoding: Encoding::Latin1,
                ..
            }
        )));
        assert_eq!(text_of(&msgs), format!("{big}é"));
        assert!(matches!(
            msgs.last(),
            Some(EdMsg::Loaded { result: Ok(()), .. })
        ));
    }

    #[test]
    fn binary_and_folders_are_blocked() {
        let mem = MemFs::new();
        mem.put_file(&uri("/b.bin"), b"\x7fELF\0\0\x01")
            .expect("put");
        mem.mkdirs(&uri("/dir")).expect("dir");
        let inbox = Inbox::default();
        load(&mem, &uri("/b.bin"), 1, 1, &inbox, &|| false);
        load(&mem, &uri("/dir"), 1, 2, &inbox, &|| false);
        let msgs = drain(&inbox);
        let blocked: Vec<&Blocked> = msgs
            .iter()
            .filter_map(|m| match m {
                EdMsg::Blocked { why, .. } => Some(why),
                _ => None,
            })
            .collect();
        assert_eq!(blocked, vec![&Blocked::Binary, &Blocked::NotAFile]);
    }

    #[test]
    fn symlinks_resolve_to_their_file() {
        let mem = MemFs::new();
        mem.put_file(&uri("/real/a.txt"), b"hi").expect("put");
        mem.put_symlink(&uri("/links/a.txt"), "../real/a.txt")
            .expect("link");
        let (real, entry) = resolve(&mem, &uri("/links/a.txt")).expect("resolve");
        assert_eq!(real, uri("/real/a.txt"));
        assert_eq!(entry.kind, EntryKind::File);
        mem.put_symlink(&uri("/loop"), "/loop").expect("loop");
        assert!(resolve(&mem, &uri("/loop")).is_err());
    }

    #[test]
    fn save_refuses_when_the_file_changed() {
        let mem = MemFs::new();
        mem.put_file(&uri("/a.txt"), b"one").expect("put");
        let before = Baseline::of(&mem.stat(&uri("/a.txt")).expect("stat"));
        mem.put_file(&uri("/a.txt"), b"other program").expect("put");
        let inbox = Inbox::default();
        let job = |expect| SaveJob {
            pane: 1,
            token: 1,
            to: uri("/a.txt"),
            buffer: Buffer::from_text("mine"),
            encoding: Encoding::Utf16Le,
            expect,
            permissions: None,
            refuse_existing: false,
        };
        save(&mem, job(Some(before)), &inbox, &|| false);
        assert!(matches!(
            drain(&inbox).first(),
            Some(EdMsg::Saved {
                result: Err(SaveFail::Conflict(Conflict::Changed)),
                ..
            })
        ));
        assert_eq!(mem.get_file(&uri("/a.txt")).expect("get"), b"other program");
        save(&mem, job(None), &inbox, &|| false);
        assert!(matches!(
            drain(&inbox).first(),
            Some(EdMsg::Saved { result: Ok(_), .. })
        ));
        assert_eq!(
            mem.get_file(&uri("/a.txt")).expect("get"),
            [0xFF, 0xFE, b'm', 0, b'i', 0, b'n', 0, b'e', 0]
        );
    }
}
