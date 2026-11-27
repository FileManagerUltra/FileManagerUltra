//! zip and zip64, with AES and ZipCrypto passwords (crate `zip`).

use std::io::Read;
use std::sync::Arc;
use std::time::SystemTime;

use fm_vfs::FmError;
use zip::ZipArchive;
use zip::read::ZipReadOptions;
use zip::result::ZipError;

use super::{FormatId, Info, Passwords, Reader, pipe, read_small};
use crate::index::{Index, Meta, NodeId, NodeKind};
use crate::limits::{INLINE_MEMBER, MAX_ENTRIES, MAX_LINK_TARGET};
use crate::source::{Source, SourceReader};

pub struct ZipReader {
    archive: ZipArchive<SourceReader>,
    index: Index,
    name: String,
    info: Info,
}

const S_IFMT: u32 = 0o170_000;
const S_IFLNK: u32 = 0o120_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFREG: u32 = 0o100_000;

impl ZipReader {
    pub fn open(src: Arc<dyn Source>, name: &str) -> Result<Self, FmError> {
        let mut archive = ZipArchive::new(SourceReader::new(src)).map_err(|e| zip_err(e, name))?;
        if archive.len() > MAX_ENTRIES {
            return Err(FmError::backend(
                "archive",
                "too many entries (limit 1 000 000)",
            ));
        }
        let mut index = Index::new();
        let mut methods: Vec<String> = Vec::new();
        let mut encrypted = false;
        let mut links = Vec::new();
        for i in 0..archive.len() {
            let entry = archive.by_index_data(i).map_err(|e| zip_err(e, name))?;
            let raw = entry
                .name()
                .map(|n| n.into_owned())
                .unwrap_or_else(|_| String::from_utf8_lossy(entry.name_raw()).into_owned());
            let mode = entry.unix_mode();
            let method = format!("{:?}", entry.compression());
            if !methods.contains(&method) {
                methods.push(method);
            }
            encrypted |= entry.encrypted();
            let meta = Meta {
                size: Some(entry.size()),
                packed: Some(entry.compressed_size()),
                modified: entry.last_modified().and_then(to_system_time),
                mode: mode.map(|m| m & 0o7777),
                uid: None,
                gid: None,
                encrypted: entry.encrypted(),
                locator: i as u64,
            };
            let kind = match mode.map(|m| m & S_IFMT) {
                _ if entry.is_dir() => NodeKind::Dir,
                Some(S_IFLNK) => NodeKind::Symlink(String::new()),
                Some(S_IFDIR) => NodeKind::Dir,
                Some(0 | S_IFREG) | None => NodeKind::File,
                Some(_) => NodeKind::Other,
            };
            let is_link = matches!(kind, NodeKind::Symlink(_));
            drop(entry);
            if let Some(id) = index.add(&raw, kind, meta)?
                && is_link
            {
                links.push((id, i));
            }
        }
        // Link targets are stored as the entry's data; read them (small, capped).
        for (id, i) in links {
            let target = archive
                .by_index(i)
                .ok()
                .and_then(|f| {
                    let mut buf = Vec::new();
                    f.take(MAX_LINK_TARGET as u64).read_to_end(&mut buf).ok()?;
                    Some(String::from_utf8_lossy(&buf).into_owned())
                })
                .unwrap_or_default();
            index.set_kind(id, NodeKind::Symlink(target));
        }
        Ok(Self {
            archive,
            index,
            name: name.to_string(),
            info: Info {
                methods: methods.into_iter().map(|m| method_label(&m)).collect(),
                encrypted,
                names_encrypted: false,
            },
        })
    }

    /// Open entry `i` with the first password that fits; the returned password (if any)
    /// is the one to use for the streaming read.
    fn check(
        &self,
        i: usize,
        encrypted: bool,
        passwords: &Passwords,
    ) -> Result<Option<Vec<u8>>, FmError> {
        let mut archive = self.archive.clone();
        if !encrypted {
            archive.by_index(i).map_err(|e| zip_err(e, &self.name))?;
            return Ok(None);
        }
        if passwords.gave_up() {
            return Err(super::gave_up_error(&self.name));
        }
        let known = passwords.list();
        if known.is_empty() {
            return Err(FmError::PasswordRequired(self.name.clone()));
        }
        for pw in known {
            match archive
                .by_index_with_options(i, ZipReadOptions::new().password(Some(pw.as_bytes())))
            {
                Ok(_) => {
                    passwords.fitted(&pw);
                    return Ok(Some(pw.as_bytes().to_vec()));
                }
                Err(ZipError::InvalidPassword) => {}
                Err(e) => return Err(zip_err(e, &self.name)),
            }
        }
        Err(passwords.failed(&self.name))
    }
}

impl Reader for ZipReader {
    fn format(&self) -> FormatId {
        FormatId::Zip
    }

    fn index(&self) -> &Index {
        &self.index
    }

    fn info(&self) -> Info {
        self.info.clone()
    }

    fn zip_archive(&self) -> Option<ZipArchive<SourceReader>> {
        Some(self.archive.clone())
    }

    fn open(&self, id: NodeId, passwords: &Passwords) -> Result<Box<dyn Read + Send>, FmError> {
        let node = self
            .index
            .node(id)
            .ok_or_else(|| FmError::NotFound(self.name.clone()))?;
        let i = usize::try_from(node.meta.locator).unwrap_or(usize::MAX);
        let size = node.meta.size.unwrap_or(0);
        let password = zeroize::Zeroizing::new(self.check(i, node.meta.encrypted, passwords)?);
        let mut archive = self.archive.clone();
        let name = self.name.clone();
        if size <= INLINE_MEMBER {
            let file = archive
                .by_index_with_options(i, ZipReadOptions::new().password(password.as_deref()))
                .map_err(|e| zip_err(e, &name))?;
            let data =
                read_small(file, size).map_err(|e| crc_hint(e, node.meta.encrypted, &name))?;
            return Ok(Box::new(std::io::Cursor::new(data)));
        }
        let encrypted = node.meta.encrypted;
        Ok(pipe(move |w| {
            let password = password;
            let file = archive
                .by_index_with_options(i, ZipReadOptions::new().password(password.as_deref()))
                .map_err(|e| zip_err(e, &name))?;
            let mut limited = super::Declared::new(file, size);
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = limited
                    .read(&mut buf)
                    .map_err(|e| crc_hint(FmError::from(e), encrypted, &name))?;
                if n == 0 {
                    return Ok(());
                }
                w.write_all(buf.get(..n).unwrap_or(&[]))?;
            }
        }))
    }
}

/// ZipCrypto's one-byte check lets some wrong passwords through; the CRC catches them.
fn crc_hint(err: FmError, encrypted: bool, name: &str) -> FmError {
    if encrypted && err.to_string().to_ascii_lowercase().contains("checksum") {
        return FmError::backend("archive", format!("Wrong password or damaged file: {name}"));
    }
    err
}

fn method_label(debug: &str) -> String {
    match debug {
        "Stored" => "stored".into(),
        "Deflated" => "Deflate".into(),
        "Deflate64" => "Deflate64".into(),
        "Bzip2" => "Bzip2".into(),
        "Lzma" => "LZMA".into(),
        "Xz" => "XZ".into(),
        "Zstd" => "Zstandard".into(),
        "Ppmd" => "PPMd".into(),
        "Aes" => "AES".into(),
        other => other.to_string(),
    }
}

pub(crate) fn zip_err(err: ZipError, name: &str) -> FmError {
    match err {
        ZipError::UnsupportedArchive(msg) if msg == ZipError::PASSWORD_REQUIRED => {
            FmError::PasswordRequired(name.to_string())
        }
        ZipError::InvalidPassword => FmError::WrongPassword(name.to_string()),
        ZipError::Io(io) => FmError::Io(io),
        ZipError::CompressionMethodNotSupported(_) => FmError::backend(
            "archive",
            format!("compression method not supported in {name}"),
        ),
        other => FmError::backend("archive", format!("{name}: {other}")),
    }
}

fn to_system_time(dt: zip::DateTime) -> Option<SystemTime> {
    crate::time::from_civil(
        i64::from(dt.year()),
        dt.month(),
        dt.day(),
        dt.hour(),
        dt.minute(),
        dt.second(),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source::MemSource;
    use std::io::{Cursor, Write};

    pub(crate) fn sample_zip() -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o644)
                .system(zip::System::Unix);
            w.add_directory("www/", opts).expect("dir");
            w.start_file("www/index.html", opts).expect("file");
            w.write_all(b"<h1>hi</h1>").expect("w");
            w.start_file("../../evil.txt", opts).expect("file");
            w.write_all(b"evil").expect("w");
            w.add_symlink("www/link", "index.html", opts).expect("link");
            w.start_file("big.bin", opts).expect("file");
            w.write_all(&vec![7u8; 3 << 20]).expect("w");
            w.finish().expect("finish");
        }
        out.into_inner()
    }

    fn open(bytes: Vec<u8>) -> ZipReader {
        ZipReader::open(Arc::new(MemSource::new(bytes)), "site.zip").expect("open")
    }

    #[test]
    fn lists_reads_and_streams() {
        let r = open(sample_zip());
        let idx = r.index();
        assert!(idx.lookup("www/index.html").is_some());
        assert!(idx.lookup("evil.txt").is_some(), "`..` clamped at the root");
        let link = idx.lookup("www/link").expect("link");
        assert_eq!(
            idx.node(link).map(|n| n.kind.clone()),
            Some(NodeKind::Symlink("index.html".into()))
        );
        let page = idx.lookup("www/index.html").expect("page");
        assert_eq!(idx.node(page).and_then(|n| n.meta.mode), Some(0o644));
        let mut s = String::new();
        r.open(page, &Passwords::default())
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "<h1>hi</h1>");
        let big = idx.lookup("big.bin").expect("big");
        let mut data = Vec::new();
        r.open(big, &Passwords::default())
            .expect("open")
            .read_to_end(&mut data)
            .expect("read");
        assert_eq!(data.len(), 3 << 20);
        assert!(r.info().methods.contains(&"Deflate".to_string()));
    }

    #[test]
    fn aes_members_need_the_right_password() {
        let mut out = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            let opts = zip::write::SimpleFileOptions::default()
                .with_aes_encryption(zip::AesMode::Aes256, "right");
            w.start_file("secret.txt", opts).expect("file");
            w.write_all(b"top secret").expect("w");
            w.finish().expect("finish");
        }
        let r = open(out.into_inner());
        let id = r.index().lookup("secret.txt").expect("id");
        assert!(r.info().encrypted);
        let pw = Passwords::default();
        assert!(matches!(r.open(id, &pw), Err(FmError::PasswordRequired(_))));
        pw.add(crate::Secret::new("wrong"));
        assert!(matches!(r.open(id, &pw), Err(FmError::WrongPassword(_))));
        pw.add(crate::Secret::new("right"));
        let mut s = String::new();
        r.open(id, &pw)
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "top secret");
    }

    fn read_all(r: &ZipReader, path: &str, pw: &Passwords) -> Result<String, FmError> {
        let id = r.index().lookup(path).expect(path);
        let mut s = String::new();
        r.open(id, pw)?.read_to_string(&mut s).expect("read");
        Ok(s)
    }

    /// A ZipCrypto archive made by Info-ZIP (`zip -P zip-crypto -r`, spike 1.6): read with
    /// its password, refused with a wrong one; its UTF-8 name is kept.
    #[test]
    fn zipcrypto_from_info_zip_reads_with_its_password() {
        let r = open(include_bytes!("fixtures/zipcrypto.zip").to_vec());
        assert!(r.info().encrypted);
        assert!(r.index().lookup("café.txt").is_some(), "UTF-8 name");
        let pw = Passwords::default();
        assert!(matches!(
            read_all(&r, "docs/legacy.txt", &pw),
            Err(FmError::PasswordRequired(_))
        ));
        pw.add(crate::Secret::new("nope"));
        assert!(
            read_all(&r, "docs/legacy.txt", &pw).is_err(),
            "wrong password"
        );
        pw.add(crate::Secret::new("zip-crypto"));
        assert_eq!(
            read_all(&r, "docs/legacy.txt", &pw).expect("read"),
            "old tools only\n"
        );
        assert_eq!(read_all(&r, "café.txt", &pw).expect("read"), "café\n");
    }

    /// AES-128, -192 and -256 members, each with its own password, in one archive: each
    /// reads once its password is known (spike 1.6, mixed passwords per member).
    #[test]
    fn aes_strengths_and_a_password_per_member() {
        let mut out = Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut out);
            for (name, mode, pw) in [
                ("a128.txt", zip::AesMode::Aes128, "one"),
                ("a192.txt", zip::AesMode::Aes192, "two"),
                ("a256.txt", zip::AesMode::Aes256, "three"),
            ] {
                let opts = zip::write::SimpleFileOptions::default().with_aes_encryption(mode, pw);
                w.start_file(name, opts).expect("file");
                w.write_all(name.as_bytes()).expect("w");
            }
            let plain = zip::write::SimpleFileOptions::default();
            w.start_file("日本語/メモ.txt", plain).expect("file");
            w.write_all(b"plain").expect("w");
            w.finish().expect("finish");
        }
        let r = open(out.into_inner());
        let pw = Passwords::default();
        assert_eq!(
            read_all(&r, "日本語/メモ.txt", &pw).expect("plain"),
            "plain"
        );
        pw.add(crate::Secret::new("two"));
        assert_eq!(read_all(&r, "a192.txt", &pw).expect("192"), "a192.txt");
        assert!(read_all(&r, "a128.txt", &pw).is_err(), "not its password");
        pw.add(crate::Secret::new("one"));
        pw.add(crate::Secret::new("three"));
        assert_eq!(read_all(&r, "a128.txt", &pw).expect("128"), "a128.txt");
        assert_eq!(read_all(&r, "a256.txt", &pw).expect("256"), "a256.txt");
    }

    #[test]
    fn garbage_is_an_error() {
        let src = Arc::new(MemSource::new(b"PK\x03\x04 not really".to_vec()));
        assert!(ZipReader::open(src, "x.zip").is_err());
    }
}
