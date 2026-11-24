//! 7z (crate `sevenz-rust2`): LZMA/LZMA2/BZip2/Deflate/PPMd, AES-256 with or without
//! encrypted names. A member is read by decoding only the solid block that holds it.

use std::io::{self, Read};
use std::sync::Arc;

use fm_vfs::FmError;
use sevenz_rust2::{Archive, BlockDecoder, Password};

use super::{FormatId, Info, Passwords, Reader, pipe};
use crate::index::{Index, Meta, NodeId, NodeKind};
use crate::limits::MAX_ENTRIES;
use crate::secret::Secret;
use crate::source::{Source, SourceReader};

/// Memory a single decoder may use (dictionary / model), in KiB: 1 GiB.
const MEM_LIMIT_KB: usize = 1 << 20;

pub struct SevenZReader {
    src: Arc<dyn Source>,
    archive: Arc<Archive>,
    index: Index,
    name: String,
    /// Password that opened encrypted headers (also tried first for the data).
    header_password: Option<Secret>,
    info: Info,
}

impl SevenZReader {
    pub fn open(src: Arc<dyn Source>, name: &str, passwords: &Passwords) -> Result<Self, FmError> {
        let mut reader = SourceReader::new(src.clone());
        let (archive, header_password) =
            match Archive::read_with_memory_limit(&mut reader, &Password::empty(), MEM_LIMIT_KB) {
                Ok(a) => (a, None),
                Err(sevenz_rust2::Error::PasswordRequired) => {
                    if passwords.gave_up() {
                        return Err(super::gave_up_error(name));
                    }
                    let known = passwords.list();
                    if known.is_empty() {
                        return Err(FmError::PasswordRequired(name.to_string()));
                    }
                    let mut found = None;
                    for pw in known {
                        let mut reader = SourceReader::new(src.clone());
                        let attempt = Archive::read_with_memory_limit(
                            &mut reader,
                            &Password::new(pw.expose()),
                            MEM_LIMIT_KB,
                        );
                        if let Ok(a) = attempt {
                            found = Some((a, Some(pw)));
                            break;
                        }
                    }
                    match found {
                        Some(f) => {
                            if let Some(pw) = &f.1 {
                                passwords.fitted(pw);
                            }
                            f
                        }
                        None => return Err(passwords.failed(name)),
                    }
                }
                Err(e) => return Err(sz_err(e, name)),
            };
        if archive.files.len() > MAX_ENTRIES {
            return Err(FmError::backend(
                "archive",
                "too many entries (limit 1 000 000)",
            ));
        }
        let mut index = Index::new();
        let mut encrypted = false;
        for (i, entry) in archive.files.iter().enumerate() {
            if entry.is_anti_item {
                continue;
            }
            let is_enc = archive.is_file_encrypted(i);
            encrypted |= is_enc;
            let meta = Meta {
                size: Some(entry.size),
                packed: Some(entry.compressed_size),
                modified: entry
                    .has_last_modified_date
                    .then(|| std::time::SystemTime::from(entry.last_modified_date)),
                mode: unix_mode(entry.windows_attributes, entry.has_windows_attributes),
                uid: None,
                gid: None,
                encrypted: is_enc,
                locator: i as u64,
            };
            let kind = if entry.is_directory {
                NodeKind::Dir
            } else {
                NodeKind::File
            };
            index.add(&entry.name, kind, meta)?;
        }
        let mut methods = Vec::new();
        for block in &archive.blocks {
            for coder in &block.coders {
                let label = method_label(coder.encoder_method_id());
                if !methods.contains(&label) {
                    methods.push(label);
                }
            }
        }
        let names_encrypted = header_password.is_some();
        Ok(Self {
            src,
            archive: Arc::new(archive),
            index,
            name: name.to_string(),
            header_password,
            info: Info {
                methods,
                encrypted: encrypted || names_encrypted,
                names_encrypted,
            },
        })
    }

    /// Decode the block of file `i` and write that file's data to `out`.
    fn extract(
        src: Arc<dyn Source>,
        archive: &Archive,
        i: usize,
        password: &Password,
        out: &mut dyn io::Write,
    ) -> Result<bool, sevenz_rust2::Error> {
        let Some(block) = archive
            .stream_map
            .file_block_index
            .get(i)
            .copied()
            .flatten()
        else {
            return Ok(true); // an empty file
        };
        let mut reader = SourceReader::new(src);
        let decoder = BlockDecoder::new_with_memory_limit(
            1,
            MEM_LIMIT_KB,
            block,
            archive,
            password,
            &mut reader,
        );
        let first = archive
            .stream_map
            .block_first_file_index
            .get(block)
            .copied()
            .unwrap_or(0);
        let mut at = first;
        let mut found = false;
        decoder.for_each_entries(&mut |_, data| {
            if at == i {
                io::copy(data, out).map_err(|e| sevenz_rust2::Error::Io(e, "write".into()))?;
                found = true;
                return Ok(false);
            }
            at += 1;
            Ok(true)
        })?;
        Ok(found)
    }
}

impl Reader for SevenZReader {
    fn format(&self) -> FormatId {
        FormatId::SevenZ
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
        let i = usize::try_from(node.meta.locator).unwrap_or(usize::MAX);
        let size = node.meta.size.unwrap_or(0);
        let mut candidates: Vec<Secret> = self.header_password.iter().cloned().collect();
        candidates.extend(passwords.list());
        let password = if node.meta.encrypted {
            if passwords.gave_up() {
                return Err(super::gave_up_error(&self.name));
            }
            if candidates.is_empty() {
                return Err(FmError::PasswordRequired(self.name.clone()));
            }
            // Find the password that decodes this block: try a tiny read with each.
            let mut ok = None;
            for pw in candidates {
                let probe = Password::new(pw.expose());
                let mut sink = Limited { left: 1 };
                match Self::extract(self.src.clone(), &self.archive, i, &probe, &mut sink) {
                    Ok(_) => {
                        ok = Some(pw);
                        break;
                    }
                    Err(e) if is_limited(&e) => {
                        ok = Some(pw);
                        break;
                    }
                    Err(_) => {}
                }
            }
            match ok {
                Some(pw) => {
                    passwords.fitted(&pw);
                    Some(pw)
                }
                None => return Err(passwords.failed(&self.name)),
            }
        } else {
            None
        };
        let src = self.src.clone();
        let archive = self.archive.clone();
        let name = self.name.clone();
        let member = pipe(move |w| {
            let pw = password
                .as_ref()
                .map_or_else(Password::empty, |s| Password::new(s.expose()));
            let found = Self::extract(src, &archive, i, &pw, w).map_err(|e| sz_err(e, &name))?;
            if !found {
                return Err(FmError::NotFound(name));
            }
            Ok(())
        });
        // Bound what comes out by the entry's size (a lying block cannot exceed it).
        Ok(Box::new(super::Declared::new(member, size)))
    }
}

/// A writer that fails after `left` bytes: used to probe a password cheaply.
struct Limited {
    left: usize,
}

impl io::Write for Limited {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other(PROBE_DONE));
        }
        let n = data.len().min(self.left);
        self.left -= n;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

const PROBE_DONE: &str = "fm-archive probe done";

fn is_limited(err: &sevenz_rust2::Error) -> bool {
    err.to_string().contains(PROBE_DONE)
}

/// 7z stores Unix modes in the high 16 bits of the attributes when bit 15 is set.
fn unix_mode(attrs: u32, has: bool) -> Option<u32> {
    (has && attrs & 0x8000 != 0).then_some((attrs >> 16) & 0o7777)
}

fn method_label(id: &[u8]) -> String {
    match id {
        [0x21] => "LZMA2".into(),
        [0x03, 0x01, 0x01] => "LZMA".into(),
        [0x04, 0x01, 0x08] => "Deflate".into(),
        [0x04, 0x02, 0x02] => "BZip2".into(),
        [0x03, 0x04, 0x01] => "PPMd".into(),
        [0x00] => "stored".into(),
        [0x06, 0xF1, 0x07, 0x01] => "AES-256".into(),
        [0x03, 0x03, 0x01, 0x1B] | [0x03, 0x03, 0x01, 0x03] => "BCJ".into(),
        [0x03] => "Delta".into(),
        _ => "other".into(),
    }
}

fn sz_err(err: sevenz_rust2::Error, name: &str) -> FmError {
    match err {
        sevenz_rust2::Error::PasswordRequired => FmError::PasswordRequired(name.to_string()),
        sevenz_rust2::Error::MaybeBadPassword(_) => FmError::WrongPassword(name.to_string()),
        sevenz_rust2::Error::UnsupportedCompressionMethod(m) => FmError::backend(
            "archive",
            format!("compression method not supported in {name}: {m}"),
        ),
        other => FmError::backend("archive", format!("{name}: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;

    fn make(password: Option<&str>, encrypt_names: bool) -> Vec<u8> {
        let mut out = io::Cursor::new(Vec::new());
        {
            let mut w = sevenz_rust2::ArchiveWriter::new(&mut out).expect("writer");
            if let Some(pw) = password {
                w.set_content_methods(vec![
                    sevenz_rust2::encoder_options::AesEncoderOptions::new(Password::new(pw)).into(),
                    sevenz_rust2::EncoderMethod::LZMA2.into(),
                ]);
                w.set_encrypt_header(encrypt_names);
            }
            for (name, data) in [
                ("docs/a.txt", &b"alpha"[..]),
                ("docs/b.txt", &b"bravo"[..]),
                ("c.bin", &[3u8; 200_000][..]),
            ] {
                let mut e = sevenz_rust2::ArchiveEntry::new_file(name);
                e.size = data.len() as u64;
                w.push_archive_entry(e, Some(data)).expect("push");
            }
            w.finish().expect("finish");
        }
        out.into_inner()
    }

    fn read(r: &SevenZReader, path: &str, pw: &Passwords) -> Result<Vec<u8>, FmError> {
        let id = r.index().lookup(path).expect(path);
        let mut out = Vec::new();
        r.open(id, pw)?.read_to_end(&mut out)?;
        Ok(out)
    }

    #[test]
    fn plain_7z_reads_each_member_of_a_solid_block() {
        let r = SevenZReader::open(
            Arc::new(MemSource::new(make(None, false))),
            "t.7z",
            &Passwords::default(),
        )
        .expect("open");
        let pw = Passwords::default();
        assert_eq!(read(&r, "docs/b.txt", &pw).expect("b"), b"bravo");
        assert_eq!(read(&r, "docs/a.txt", &pw).expect("a"), b"alpha");
        assert_eq!(read(&r, "c.bin", &pw).expect("c").len(), 200_000);
        assert!(!r.info().encrypted);
    }

    #[test]
    fn encrypted_names_need_the_password_to_mount() {
        let bytes = make(Some("s3cret"), true);
        let src: Arc<dyn Source> = Arc::new(MemSource::new(bytes));
        let pw = Passwords::default();
        assert!(matches!(
            SevenZReader::open(src.clone(), "s.7z", &pw),
            Err(FmError::PasswordRequired(_))
        ));
        pw.add(Secret::new("nope"));
        assert!(matches!(
            SevenZReader::open(src.clone(), "s.7z", &pw),
            Err(FmError::WrongPassword(_))
        ));
        pw.add(Secret::new("s3cret"));
        let r = SevenZReader::open(src, "s.7z", &pw).expect("open");
        assert!(r.info().names_encrypted);
        assert_eq!(
            read(&r, "docs/a.txt", &Passwords::default()).expect("a"),
            b"alpha"
        );
    }

    #[test]
    fn encrypted_data_asks_when_read() {
        let r = SevenZReader::open(
            Arc::new(MemSource::new(make(Some("pw"), false))),
            "d.7z",
            &Passwords::default(),
        )
        .expect("open");
        let pw = Passwords::default();
        assert!(matches!(
            read(&r, "docs/a.txt", &pw),
            Err(FmError::PasswordRequired(_))
        ));
        pw.add(Secret::new("bad"));
        assert!(matches!(
            read(&r, "docs/a.txt", &pw),
            Err(FmError::WrongPassword(_))
        ));
        pw.add(Secret::new("pw"));
        assert_eq!(read(&r, "docs/a.txt", &pw).expect("a"), b"alpha");
    }
}
