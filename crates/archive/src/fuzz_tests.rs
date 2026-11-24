//! Damaged-archive corpus (add-archive-vfs 3.11): every readable format, truncated at
//! many lengths, bit-flipped and with header fields forced to their maximum, is opened
//! and every member read through [`ArchiveFs`] exactly as a pane or an extraction would.
//! Nothing may panic past the decoder guards, and no read may produce more than the cap.

use std::io::{self, Read};
use std::sync::Arc;

use fm_vfs::{Vfs, VfsUri};

use crate::Secret;
use crate::format::{FormatId, Passwords, Progress, guarded, open};
use crate::fs::ArchiveFs;
use crate::source::MemSource;
use crate::write::{CompressOptions, Hooks, Level, VfsFor, compress, walk};

/// Most bytes read from one member of a damaged archive.
const READ_CAP: u64 = 64 << 20;

/// A small tree in `format` (optionally encrypted), written by our own writers.
fn fixture(format: FormatId, password: Option<&str>) -> Vec<u8> {
    let mem = Arc::new(fm_testkit::MemFs::new());
    mem.mkdirs_str("/p/docs/empty").expect("dirs");
    mem.put_str("/p/docs/a.txt", b"alpha alpha alpha")
        .expect("a");
    mem.put_str("/p/notes.txt", &b"notes ".repeat(200))
        .expect("n");
    mem.put_str("/p/zeros.bin", &[0u8; 20_000]).expect("z");
    let vfs: Arc<dyn Vfs> = mem;
    let vfs_for: VfsFor = Arc::new(move |_| Ok(vfs.clone()));
    let sources: Vec<VfsUri> = ["/p/docs", "/p/notes.txt", "/p/zeros.bin"]
        .iter()
        .map(|s| VfsUri::parse(s).expect("uri"))
        .collect();
    let w = walk(&sources, &vfs_for, &|| false).expect("walk");
    let opts = CompressOptions {
        format,
        level: Level::Fast,
        password: password.map(Secret::new),
        zip_crypto: false,
        encrypt_names: true,
    };
    let mut progress = |_: u64, _: &str| {};
    let mut hooks = Hooks {
        progress: &mut progress,
        cancelled: &|| false,
    };
    compress(io::Cursor::new(Vec::new()), &w, &vfs_for, &opts, &mut hooks)
        .expect("compress")
        .into_inner()
}

fn gz(data: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    enc.write_all(data).expect("gz");
    enc.finish().expect("gz")
}

#[derive(Default, Debug)]
struct Tally {
    opened: usize,
    refused: usize,
    reads: usize,
    /// Decoder panics caught by the guards (they became errors, not crashes).
    caught: usize,
}

fn is_caught(e: &impl ToString) -> bool {
    e.to_string().contains("decoder failure")
}

/// Open `bytes` as `format` and read every member; count what happened.
fn exercise(bytes: Vec<u8>, format: FormatId, password: Option<&str>, tally: &mut Tally) {
    let passwords = Arc::new(Passwords::default());
    if let Some(p) = password {
        passwords.add(Secret::new(p));
    }
    let src = Arc::new(MemSource::new(bytes));
    let reader = match guarded(|| open(src, format, "fuzz", &passwords, &Progress::none())) {
        Ok(r) => r,
        Err(e) => {
            tally.caught += usize::from(is_caught(&e));
            tally.refused += 1;
            return;
        }
    };
    tally.opened += 1;
    let root = VfsUri::archive_root("00000000000000aa").expect("root");
    let fs = ArchiveFs::new(root.clone(), reader, passwords);
    for (_, path) in fs.index().files() {
        let mut uri = root.clone();
        let mut ok = true;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            match uri.join(part) {
                Ok(u) => uri = u,
                Err(_) => ok = false,
            }
        }
        if !ok {
            continue;
        }
        let r = match fs.open_read(&uri) {
            Ok(r) => r,
            Err(e) => {
                tally.caught += usize::from(is_caught(&e));
                continue;
            }
        };
        tally.reads += 1;
        match io::copy(&mut r.take(READ_CAP + 1), &mut io::sink()) {
            Ok(n) => assert!(n <= READ_CAP, "{format:?}: {path} read {n} bytes"),
            Err(e) => tally.caught += usize::from(is_caught(&e)),
        }
    }
}

/// Truncations, single-bit flips and 0xFF runs (oversized counts, sizes and offsets)
/// spread over the whole file.
fn mutations(base: &[u8]) -> Vec<Vec<u8>> {
    let len = base.len();
    let mut out = Vec::new();
    let step = |n: usize| (len / n).max(1);
    for cut in (0..len).step_by(step(120)) {
        out.push(base.get(..cut).unwrap_or_default().to_vec());
    }
    for (i, at) in (0..len).step_by(step(160)).enumerate() {
        let mut m = base.to_vec();
        if let Some(b) = m.get_mut(at) {
            *b ^= 1 << (i % 8);
        }
        out.push(m);
    }
    for at in (0..len).step_by(step(80)) {
        let mut m = base.to_vec();
        for b in m.iter_mut().skip(at).take(8) {
            *b = 0xFF;
        }
        out.push(m);
    }
    out
}

fn run_corpus(format: FormatId, base: &[u8], password: Option<&str>) -> Tally {
    let mut tally = Tally::default();
    // The intact archive must open and read cleanly, or the corpus proves nothing.
    exercise(base.to_vec(), format, password, &mut tally);
    assert_eq!((tally.opened, tally.caught), (1, 0), "{format:?} intact");
    assert!(tally.reads > 0, "{format:?} intact has members");
    for m in mutations(base) {
        exercise(m, format, password, &mut tally);
    }
    assert!(tally.refused > 0, "{format:?}: some damage must be noticed");
    // The guards would turn a decoder panic into an error, but none should be needed:
    // a new one after a dependency update is worth a look.
    assert_eq!(tally.caught, 0, "{format:?}: decoder panics {tally:?}");
    tally
}

#[test]
fn damaged_archives_of_every_format_never_crash() {
    let cases: Vec<(FormatId, Option<&str>)> = vec![
        (FormatId::Zip, None),
        (FormatId::Zip, Some("pw")),
        (FormatId::SevenZ, None),
        (FormatId::Tar, None),
        (FormatId::TarGz, None),
        (FormatId::TarBz2, None),
        (FormatId::TarXz, None),
    ];
    for (format, password) in cases {
        let base = fixture(format, password);
        let tally = run_corpus(format, &base, password);
        eprintln!("{format:?} pw={}: {tally:?}", password.is_some());
    }
    let single = gz(&b"log line\n".repeat(500));
    let tally = run_corpus(FormatId::Gz, &single, None);
    eprintln!("Gz: {tally:?}");
}

/// An encrypted 7z (names hidden) opened without its password: the header parser sees
/// damage before any key is derived, which keeps this fast.
#[test]
fn damaged_encrypted_7z_headers_never_crash() {
    let base = fixture(FormatId::SevenZ, Some("pw"));
    let mut tally = Tally::default();
    for m in mutations(&base) {
        exercise(m, FormatId::SevenZ, None, &mut tally);
    }
    eprintln!("7z encrypted: {tally:?}");
    assert!(tally.refused > 0);
    assert_eq!(tally.caught, 0, "decoder panics {tally:?}");
}

/// A tar header whose size field claims the largest octal value, with a valid checksum:
/// the index must refuse or bound it, never allocate it.
#[test]
fn a_tar_size_field_at_its_maximum_is_refused_or_bounded() {
    let mut base = fixture(FormatId::Tar, None);
    let header = base.get_mut(..512).expect("header");
    header
        .get_mut(124..136)
        .expect("size")
        .copy_from_slice(b"77777777777\0");
    header.get_mut(148..156).expect("cksum").fill(b' ');
    let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
    let cksum = format!("{sum:06o}\0 ");
    header
        .get_mut(148..156)
        .expect("cksum")
        .copy_from_slice(cksum.as_bytes());
    let mut tally = Tally::default();
    exercise(base.clone(), FormatId::Tar, None, &mut tally);
    exercise(gz(&base), FormatId::TarGz, None, &mut tally);
    eprintln!("tar max size: {tally:?}");
    assert_eq!(tally.caught, 0, "decoder panics {tally:?}");
}
