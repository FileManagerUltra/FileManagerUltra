//! Writing an archive again with its staged changes (design D7). Zip copies unchanged
//! members as stored (no recompression, encryption kept) and compresses only new or
//! replaced files; the tar family and 7z are written whole from the merged view. An
//! encrypted archive stays encrypted with the password that opens it. [`verify`] reads
//! the result back before the engine puts it in place.

use std::io::{self, Seek, Write};
use std::sync::Arc;

use fm_vfs::{FmError, Vfs};

use crate::Secret;
use crate::format::{self, FormatId, Passwords, Progress};
use crate::fs::ArchiveFs;
use crate::index::NodeKind;
use crate::source::Source;
use crate::write::{
    CompressOptions, Counted, Hooks, Level, VfsFor, Walk, WalkKind, compress, io_err, walk,
    zip_err, zip_time,
};

/// What the saved archive must hold, checked by [`verify`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expect {
    pub files: usize,
    pub bytes: u64,
}

/// The merged view (archive plus staged changes) as items to store.
pub fn plan(fs: &Arc<ArchiveFs>, cancelled: &dyn Fn() -> bool) -> Result<Walk, FmError> {
    let me: Arc<dyn Vfs> = fs.clone();
    let vfs_for: VfsFor = Arc::new(move |_| Ok(me.clone()));
    let mut top = fs.read_dir(fs.root())?;
    top.sort_by(|a, b| a.name.cmp(&b.name));
    let sources: Vec<_> = top.into_iter().map(|e| e.uri).collect();
    walk(&sources, &vfs_for, cancelled)
}

fn expect_of(w: &Walk) -> Expect {
    let files = w.items.iter().filter(|i| i.kind == WalkKind::File);
    Expect {
        files: files.clone().count(),
        bytes: files.map(|i| i.size).sum(),
    }
}

/// The known password that opens this archive's encrypted members.
fn fitting_password(fs: &ArchiveFs) -> Result<Secret, FmError> {
    let name = "this archive".to_string();
    let known = fs.passwords().list();
    if known.is_empty() {
        return Err(FmError::PasswordRequired(name));
    }
    let index = fs.index();
    let probe = index
        .files()
        .into_iter()
        .find(|(id, _)| index.node(*id).is_some_and(|n| n.meta.encrypted));
    let Some((id, _)) = probe else {
        // Only the names are encrypted (7z): the archive opened with the newest one.
        return known
            .into_iter()
            .next()
            .ok_or(FmError::PasswordRequired(name));
    };
    for pw in known {
        let one = Passwords::default();
        one.add(pw.clone());
        let fits = format::guarded(|| fs.reader().open(id, &one))
            .and_then(|mut r| io::Read::read(&mut r, &mut [0u8; 1]).map_err(FmError::Io))
            .is_ok();
        if fits {
            return Ok(pw);
        }
    }
    Err(FmError::WrongPassword(name))
}

/// Write the archive with its staged changes to `out`.
pub fn save<W: Write + Seek>(
    fs: &Arc<ArchiveFs>,
    out: W,
    hooks: &mut Hooks<'_>,
) -> Result<(W, Expect), FmError> {
    if let Some(why) = fs.writable().err().filter(|_| !fs.is_saving()) {
        return Err(why);
    }
    let w = plan(fs, hooks.cancelled)?;
    let expect = expect_of(&w);
    let info = fs.info();
    // A staged password change re-encrypts (or decrypts) every member; otherwise the
    // archive keeps the password that opens it.
    let (password, encrypt_names, keep_stored) = match fs.password_change() {
        Some(change) => {
            let names = change.encrypt_names && change.new.is_some();
            (change.new, names, false)
        }
        None if info.encrypted || info.names_encrypted => {
            (Some(fitting_password(fs)?), info.names_encrypted, true)
        }
        None => (None, false, true),
    };
    let me: Arc<dyn Vfs> = fs.clone();
    let vfs_for: VfsFor = Arc::new(move |_| Ok(me.clone()));
    let out = match fs.format() {
        FormatId::Zip => save_zip(out, &w, fs, &vfs_for, password, keep_stored, hooks)?,
        format => {
            let opts = CompressOptions {
                format,
                level: Level::Normal,
                password,
                zip_crypto: false,
                encrypt_names,
            };
            compress(out, &w, &vfs_for, &opts, hooks)?
        }
    };
    Ok((out, expect))
}

fn save_zip<W: Write + Seek>(
    out: W,
    w: &Walk,
    fs: &ArchiveFs,
    vfs_for: &VfsFor,
    password: Option<Secret>,
    keep_stored: bool,
    hooks: &mut Hooks<'_>,
) -> Result<W, FmError> {
    let mut stored = fs.reader().zip_archive().filter(|_| keep_stored);
    let mut zw = zip::ZipWriter::new(out);
    let mut done = 0u64;
    for item in &w.items {
        if (hooks.cancelled)() {
            return Err(FmError::Cancelled);
        }
        let mut o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip_time(item.modified))
            .large_file(item.size >= u64::from(u32::MAX));
        if let Some(mode) = item.mode {
            // Mark the entry as Unix-made: on Windows the zip crate defaults to DOS,
            // and readers then ignore the stored permission bits.
            o = o.unix_permissions(mode & 0o7777).system(zip::System::Unix);
        }
        match &item.kind {
            WalkKind::Dir => zw
                .add_directory(format!("{}/", item.rel), o)
                .map_err(zip_err)?,
            WalkKind::Symlink(target) => zw.add_symlink(&item.rel, target, o).map_err(zip_err)?,
            WalkKind::File => {
                let member = fs
                    .base_member(&item.uri)
                    .and_then(|id| fs.index().node(id))
                    .filter(|n| n.kind == NodeKind::File)
                    .and_then(|n| usize::try_from(n.meta.locator).ok());
                if let (Some(i), Some(src)) = (member, stored.as_mut()) {
                    let file = src.by_index_raw(i).map_err(zip_err)?;
                    zw.raw_copy_file_rename(file, &item.rel).map_err(zip_err)?;
                    done = done.saturating_add(item.size);
                    (hooks.progress)(done, &item.rel);
                    continue;
                }
                let o = match &password {
                    Some(pw) => o.with_aes_encryption(zip::AesMode::Aes256, pw.expose()),
                    None => o,
                };
                zw.start_file(&item.rel, o).map_err(zip_err)?;
                let mut data = Counted {
                    inner: vfs_for(&item.uri)?.open_read(&item.uri)?,
                    hooks,
                    done: &mut done,
                    name: &item.rel,
                };
                io::copy(&mut data, &mut zw).map_err(io_err)?;
            }
        }
    }
    zw.finish().map_err(zip_err)
}

/// The passwords that open the archive as [`save`] writes it.
pub fn saved_passwords(fs: &ArchiveFs) -> Arc<Passwords> {
    match fs.password_change() {
        Some(change) => {
            let p = Passwords::default();
            if let Some(new) = change.new {
                p.add(new);
            }
            Arc::new(p)
        }
        None => fs.passwords().clone(),
    }
}

/// After saving over the archive: its mount knows the new password (or none).
pub fn adopt_password(fs: &ArchiveFs) {
    if let Some(change) = fs.password_change() {
        fs.passwords().forget();
        if let Some(new) = change.new {
            fs.passwords().add(new);
        }
    }
}

/// Open the written archive and check that it holds what was planned.
pub fn verify(
    src: Arc<dyn Source>,
    format: FormatId,
    passwords: &Passwords,
    expect: &Expect,
) -> Result<(), FmError> {
    let reader = format::guarded(|| {
        format::open(src, format, "saved archive", passwords, &Progress::none())
    })?;
    let index = reader.index();
    let mut got = Expect { files: 0, bytes: 0 };
    for (id, _) in index.files() {
        if let Some(n) = index.node(id).filter(|n| n.kind == NodeKind::File) {
            got.files += 1;
            got.bytes = got.bytes.saturating_add(n.meta.size.unwrap_or(0));
        }
    }
    if got != *expect {
        return Err(FmError::backend(
            "archive",
            format!(
                "the saved archive did not read back correctly ({} of {} files, {} of {} bytes)",
                got.files, expect.files, got.bytes, expect.bytes
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use fm_vfs::{VfsUri, WriteOpts};
    use std::io::{Cursor, Read as _};

    fn fixture(format: FormatId, password: Option<&str>) -> Vec<u8> {
        let mem = Arc::new(fm_testkit::MemFs::new());
        mem.mkdirs_str("/p/docs").expect("dirs");
        mem.put_str("/p/docs/a.txt", b"alpha").expect("a");
        mem.put_str("/p/readme", b"hello").expect("r");
        let vfs: Arc<dyn Vfs> = mem;
        let vfs_for: VfsFor = Arc::new(move |_| Ok(vfs.clone()));
        let sources: Vec<VfsUri> = ["/p/docs", "/p/readme"]
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
        compress(Cursor::new(Vec::new()), &w, &vfs_for, &opts, &mut hooks)
            .expect("compress")
            .into_inner()
    }

    fn mount(bytes: Vec<u8>, format: FormatId, password: Option<&str>) -> Arc<ArchiveFs> {
        let passwords = Arc::new(Passwords::default());
        if let Some(p) = password {
            passwords.add(Secret::new(p));
        }
        let src = Arc::new(MemSource::new(bytes));
        let reader = format::open(src, format, "t", &passwords, &Progress::none()).expect("open");
        let root = VfsUri::archive_root("00000000000000ab").expect("root");
        Arc::new(ArchiveFs::new(root, reader, passwords))
    }

    fn put(fs: &ArchiveFs, path: &str, data: &[u8]) {
        let mut uri = fs.root().clone();
        for p in path.split('/') {
            uri = uri.join(p).expect("join");
        }
        let mut w = fs.open_write(&uri, WriteOpts::default()).expect("write");
        w.write_all(data).expect("data");
        w.flush().expect("flush");
    }

    fn uri(fs: &ArchiveFs, path: &str) -> VfsUri {
        path.split('/')
            .fold(fs.root().clone(), |u, p| u.join(p).expect("join"))
    }

    fn save_to_vec(fs: &Arc<ArchiveFs>) -> (Vec<u8>, Expect) {
        let mut progress = |_: u64, _: &str| {};
        let mut hooks = Hooks {
            progress: &mut progress,
            cancelled: &|| false,
        };
        let (out, expect) = save(fs, Cursor::new(Vec::new()), &mut hooks).expect("save");
        (out.into_inner(), expect)
    }

    fn read(fs: &ArchiveFs, path: &str) -> String {
        let mut s = String::new();
        fs.open_read(&uri(fs, path))
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        s
    }

    #[test]
    fn every_writable_format_saves_its_staged_changes() {
        for format in [
            FormatId::Zip,
            FormatId::Tar,
            FormatId::TarGz,
            FormatId::TarXz,
            FormatId::TarBz2,
            FormatId::SevenZ,
        ] {
            let fs = mount(fixture(format, None), format, None);
            put(&fs, "docs/new.txt", b"new file");
            fs.rename(&uri(&fs, "readme"), &uri(&fs, "README.md"))
                .expect("rename");
            fs.remove(&uri(&fs, "docs/a.txt")).expect("rm");
            fs.create_dir(&uri(&fs, "empty")).expect("mkdir");
            let (bytes, expect) = save_to_vec(&fs);
            assert_eq!(
                expect,
                Expect {
                    files: 2,
                    bytes: 13
                },
                "{format:?}"
            );
            verify(
                Arc::new(MemSource::new(bytes.clone())),
                format,
                &Passwords::default(),
                &expect,
            )
            .expect("verify");
            let saved = mount(bytes, format, None);
            assert_eq!(read(&saved, "docs/new.txt"), "new file", "{format:?}");
            assert_eq!(read(&saved, "README.md"), "hello", "{format:?}");
            assert!(
                saved.stat(&uri(&saved, "docs/a.txt")).is_err(),
                "{format:?}"
            );
            assert!(
                saved.stat(&uri(&saved, "empty")).expect("dir").is_dir(),
                "{format:?}"
            );
        }
    }

    #[test]
    fn unchanged_zip_members_are_copied_as_stored() {
        let original = fixture(FormatId::Zip, None);
        let fs = mount(original.clone(), FormatId::Zip, None);
        put(&fs, "extra.txt", b"x");
        let (bytes, _) = save_to_vec(&fs);
        let mut a = zip::ZipArchive::new(Cursor::new(original)).expect("a");
        let mut b = zip::ZipArchive::new(Cursor::new(bytes)).expect("b");
        let before = a.by_name("readme").expect("r");
        let after = b.by_name("readme").expect("r");
        assert_eq!(
            (before.crc32(), before.compressed_size()),
            (after.crc32(), after.compressed_size())
        );
        assert_eq!(before.last_modified(), after.last_modified());
    }

    #[test]
    fn encrypted_archives_stay_encrypted_with_their_password() {
        for format in [FormatId::Zip, FormatId::SevenZ] {
            let fs = mount(fixture(format, Some("pw")), format, Some("pw"));
            put(&fs, "added.txt", b"secret too");
            let (bytes, expect) = save_to_vec(&fs);
            let with = Passwords::default();
            with.add(Secret::new("pw"));
            verify(
                Arc::new(MemSource::new(bytes.clone())),
                format,
                &with,
                &expect,
            )
            .expect("verify");
            let saved = mount(bytes.clone(), format, Some("pw"));
            assert_eq!(read(&saved, "added.txt"), "secret too");
            // Without the password nothing new is readable.
            let src = Arc::new(MemSource::new(bytes));
            if let Ok(r) = format::open(src, format, "t", &Passwords::default(), &Progress::none())
            {
                let id = r.index().lookup("added.txt").expect("added");
                assert!(
                    r.open(id, &Passwords::default())
                        .and_then(|mut x| x.read(&mut [0u8; 4]).map_err(FmError::Io))
                        .is_err(),
                    "{format:?}"
                );
            }
        }
        // No password known: Save asks for it instead of writing an unencrypted copy.
        let fs = mount(fixture(FormatId::Zip, Some("pw")), FormatId::Zip, None);
        put(&fs, "x", b"x");
        let mut progress = |_: u64, _: &str| {};
        let mut hooks = Hooks {
            progress: &mut progress,
            cancelled: &|| false,
        };
        assert!(matches!(
            save(&fs, Cursor::new(Vec::new()), &mut hooks),
            Err(FmError::PasswordRequired(_))
        ));
    }

    #[test]
    fn a_staged_password_is_set_changed_or_removed_on_save() {
        use crate::overlay::PasswordChange;
        // Set: every member of a plain zip is AES-encrypted afterwards.
        let fs = mount(fixture(FormatId::Zip, None), FormatId::Zip, None);
        fs.stage_password(PasswordChange {
            new: Some(Secret::new("new")),
            encrypt_names: false,
        })
        .expect("stage");
        assert!(fs.is_dirty(), "a password change alone is a change");
        assert_eq!(fs.changes().first().map(|c| c.kind), Some("password"));
        let (bytes, expect) = save_to_vec(&fs);
        verify(
            Arc::new(MemSource::new(bytes.clone())),
            FormatId::Zip,
            &saved_passwords(&fs),
            &expect,
        )
        .expect("verify with the new password");
        let mut z = zip::ZipArchive::new(Cursor::new(bytes.clone())).expect("zip");
        for i in 0..z.len() {
            let e = z.by_index_raw(i).expect("entry");
            assert!(
                e.is_dir() || e.encrypted(),
                "{}",
                e.name_raw().escape_ascii()
            );
        }
        let saved = mount(bytes, FormatId::Zip, Some("new"));
        assert_eq!(read(&saved, "readme"), "hello");

        // Remove: an encrypted 7z with hidden names opens without a password afterwards.
        let fs = mount(
            fixture(FormatId::SevenZ, Some("pw")),
            FormatId::SevenZ,
            Some("pw"),
        );
        fs.stage_password(PasswordChange {
            new: None,
            encrypt_names: false,
        })
        .expect("stage");
        let (bytes, _) = save_to_vec(&fs);
        let saved = mount(bytes, FormatId::SevenZ, None);
        assert_eq!(read(&saved, "docs/a.txt"), "alpha");
        assert!(!saved.info().encrypted && !saved.info().names_encrypted);

        // Change: the old password no longer opens a 7z, the new one does (names hidden).
        let fs = mount(
            fixture(FormatId::SevenZ, Some("pw")),
            FormatId::SevenZ,
            Some("pw"),
        );
        fs.stage_password(PasswordChange {
            new: Some(Secret::new("other")),
            encrypt_names: true,
        })
        .expect("stage");
        let (bytes, _) = save_to_vec(&fs);
        let old = Passwords::default();
        old.add(Secret::new("pw"));
        assert!(
            format::open(
                Arc::new(MemSource::new(bytes.clone())),
                FormatId::SevenZ,
                "t",
                &old,
                &Progress::none()
            )
            .is_err()
        );
        let saved = mount(bytes, FormatId::SevenZ, Some("other"));
        assert_eq!(read(&saved, "readme"), "hello");

        // Tar has no encryption; an empty password is refused.
        let fs = mount(fixture(FormatId::TarGz, None), FormatId::TarGz, None);
        assert!(
            fs.stage_password(PasswordChange {
                new: Some(Secret::new("x")),
                encrypt_names: false
            })
            .is_err()
        );
        let fs = mount(fixture(FormatId::Zip, None), FormatId::Zip, None);
        assert!(
            fs.stage_password(PasswordChange {
                new: Some(Secret::new("")),
                encrypt_names: false
            })
            .is_err()
        );
        fs.stage_password(PasswordChange {
            new: None,
            encrypt_names: false,
        })
        .expect("no-op");
        assert!(
            !fs.is_dirty(),
            "removing a password it never had changes nothing"
        );
    }

    #[test]
    fn cancel_and_a_bad_result_are_errors() {
        let fs = mount(fixture(FormatId::TarGz, None), FormatId::TarGz, None);
        put(&fs, "big", &vec![7u8; 300_000]);
        let mut progress = |_: u64, _: &str| {};
        let mut hooks = Hooks {
            progress: &mut progress,
            cancelled: &|| true,
        };
        assert!(matches!(
            save(&fs, Cursor::new(Vec::new()), &mut hooks),
            Err(FmError::Cancelled)
        ));
        let (bytes, mut expect) = save_to_vec(&fs);
        expect.files += 1;
        assert!(
            verify(
                Arc::new(MemSource::new(bytes)),
                FormatId::TarGz,
                &Passwords::default(),
                &expect
            )
            .is_err()
        );
    }
}
