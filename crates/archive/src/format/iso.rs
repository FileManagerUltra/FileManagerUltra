//! ISO 9660 images, read-only (design D3/D13), with our own small reader instead of a
//! young dependency: the primary volume's tree with Rock Ridge names, modes and links
//! when present, else the Joliet (UCS-2) tree, else plain ISO names (`NAME.EXT;1` →
//! `NAME.EXT`). Multi-extent files are joined. Everything read from the image is bounded:
//! records never cross sectors, continuation areas and directory extents are checked
//! against the image size, each directory is visited once (no loops), and the index
//! limits apply. UDF-only discs show only what their ISO 9660 bridge holds.

use std::collections::HashSet;
use std::io::Read;
use std::sync::Arc;

use fm_vfs::FmError;

use super::{FormatId, Info, Passwords, Progress, Reader};
use crate::index::{Index, Meta, NodeId, NodeKind};
use crate::source::{ConcatSource, SliceSource, Source, SourceReader};

const SECTOR: u64 = 2048;
/// Directory records walked in one image (all directories together).
const MAX_RECORDS: usize = 2_000_000;
/// Largest single directory extent read.
const MAX_DIR_BYTES: u64 = 64 << 20;
/// Rock Ridge continuation areas followed per record.
const MAX_CE: usize = 16;

fn bad(what: &str) -> FmError {
    FmError::backend("archive", format!("the disc image is damaged ({what})"))
}

fn read_at(src: &dyn Source, at: u64, len: usize) -> Result<Vec<u8>, FmError> {
    if at.saturating_add(len as u64) > src.len() {
        return Err(bad("an area lies past the end of the image"));
    }
    let mut buf = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = src.read_at(at + done as u64, buf.get_mut(done..).unwrap_or(&mut []))?;
        if n == 0 {
            return Err(bad("short read"));
        }
        done += n;
    }
    Ok(buf)
}

fn u32le(b: &[u8], at: usize) -> u32 {
    b.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn u16le(b: &[u8], at: usize) -> u16 {
    b.get(at..at + 2)
        .and_then(|s| s.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

/// A directory record's fields.
#[derive(Clone, Debug)]
struct Record {
    extent: u32,
    size: u32,
    flags: u8,
    name: Vec<u8>,
    modified: Option<std::time::SystemTime>,
    system_use: Vec<u8>,
}

impl Record {
    fn is_dir(&self) -> bool {
        self.flags & 0x02 != 0
    }

    /// More extents of this file follow in the next record.
    fn continues(&self) -> bool {
        self.flags & 0x80 != 0
    }

    fn is_dot(&self) -> bool {
        matches!(self.name.as_slice(), [0] | [1])
    }
}

fn parse_record(b: &[u8]) -> Option<Record> {
    let len = usize::from(*b.first()?);
    if len < 34 || len > b.len() {
        return None;
    }
    let name_len = usize::from(*b.get(32)?);
    let name = b.get(33..33 + name_len)?.to_vec();
    let su_start = 33 + name_len + usize::from(name_len.is_multiple_of(2));
    let system_use = b.get(su_start.min(len)..len).unwrap_or_default().to_vec();
    let &[year, month, day, hour, minute, second, gmt] = b.get(18..25)? else {
        return None;
    };
    let modified =
        crate::time::from_civil(1900 + i64::from(year), month, day, hour, minute, second).map(
            |m| {
                // Offset from GMT in 15-minute steps (signed).
                let off = i64::from(gmt as i8) * 15 * 60;
                if off >= 0 {
                    m.checked_sub(std::time::Duration::from_secs(off.unsigned_abs()))
                        .unwrap_or(m)
                } else {
                    m.checked_add(std::time::Duration::from_secs(off.unsigned_abs()))
                        .unwrap_or(m)
                }
            },
        );
    Some(Record {
        extent: u32le(b, 2),
        size: u32le(b, 10),
        flags: *b.get(25)?,
        name,
        modified,
        system_use,
    })
}

/// Rock Ridge facts from a record's system use area (and its continuations).
#[derive(Default, Debug)]
struct RockRidge {
    name: Option<String>,
    mode: Option<u32>,
    link: Option<String>,
    /// `RE`: this directory was relocated; its real place is a `CL` elsewhere (skipped).
    relocated: bool,
}

fn rock_ridge(src: &dyn Source, su: &[u8], skip: usize) -> RockRidge {
    let mut rr = RockRidge::default();
    let mut name = Vec::new();
    let mut link_parts: Vec<String> = Vec::new();
    let mut areas: Vec<Vec<u8>> = vec![su.get(skip..).unwrap_or_default().to_vec()];
    let mut followed = 0;
    while let Some(area) = areas.pop() {
        let mut i = 0;
        while i + 4 <= area.len() {
            let sig = area.get(i..i + 2).unwrap_or_default();
            let len = usize::from(area.get(i + 2).copied().unwrap_or(0));
            if len < 4 || i + len > area.len() {
                break;
            }
            let body = area.get(i + 4..i + len).unwrap_or_default();
            match sig {
                b"NM" => {
                    // flags: 1 continue, 2 current dir, 4 parent dir.
                    if body.first().is_some_and(|f| f & 0x06 == 0) {
                        name.extend_from_slice(body.get(1..).unwrap_or_default());
                    }
                }
                b"PX" => rr.mode = Some(u32le(body, 0) & 0o7777),
                b"RE" => rr.relocated = true,
                b"SL" => {
                    // Component records: flags, length, content.
                    let mut j = 1;
                    while j + 2 <= body.len() {
                        let flags = body.get(j).copied().unwrap_or(0);
                        let clen = usize::from(body.get(j + 1).copied().unwrap_or(0));
                        let content = body.get(j + 2..j + 2 + clen).unwrap_or_default();
                        let part = if flags & 0x02 != 0 {
                            ".".to_string()
                        } else if flags & 0x04 != 0 {
                            "..".to_string()
                        } else if flags & 0x08 != 0 {
                            String::new()
                        } else {
                            String::from_utf8_lossy(content).into_owned()
                        };
                        link_parts.push(part);
                        j += 2 + clen;
                    }
                }
                b"CE" if followed < MAX_CE => {
                    followed += 1;
                    let block = u64::from(u32le(body, 0));
                    let offset = u64::from(u32le(body, 8));
                    let length = u32le(body, 16).min(SECTOR as u32) as usize;
                    if let Ok(more) = read_at(src, block * SECTOR + offset, length) {
                        areas.push(more);
                    }
                }
                _ => {}
            }
            i += len;
        }
    }
    if !name.is_empty() {
        rr.name = Some(String::from_utf8_lossy(&name).into_owned());
    }
    if !link_parts.is_empty() {
        let joined = link_parts.join("/");
        rr.link = Some(
            if joined.starts_with('/') || link_parts.first().is_some_and(String::is_empty) {
                format!("/{}", joined.trim_start_matches('/'))
            } else {
                joined
            },
        );
    }
    rr
}

/// `NAME.EXT;1` → `NAME.EXT`, `DIR.` → `DIR`; Joliet names are UCS-2 big-endian.
fn plain_name(raw: &[u8], joliet: bool) -> String {
    let s = if joliet {
        let units: Vec<u16> = raw
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    let s = match s.rsplit_once(';') {
        Some((n, v)) if v.bytes().all(|b| b.is_ascii_digit()) => n.to_string(),
        _ => s,
    };
    s.strip_suffix('.').map_or(s.clone(), str::to_string)
}

/// Which tree to read and how its names look.
struct Tree {
    root: Record,
    joliet: bool,
}

fn volume_trees(src: &dyn Source) -> Result<(Option<Tree>, Option<Tree>), FmError> {
    let mut primary = None;
    let mut joliet = None;
    for n in 16..16 + 64u64 {
        let vd = read_at(src, n * SECTOR, SECTOR as usize)?;
        if vd.get(1..6) != Some(b"CD001") {
            return Err(bad("no volume descriptor"));
        }
        match vd.first() {
            Some(1) if primary.is_none() => {
                if u16le(&vd, 128) != SECTOR as u16 {
                    return Err(FmError::backend(
                        "archive",
                        "disc images with blocks other than 2048 bytes are not supported",
                    ));
                }
                let root = parse_record(vd.get(156..190).unwrap_or_default())
                    .ok_or_else(|| bad("root"))?;
                primary = Some(Tree {
                    root,
                    joliet: false,
                });
            }
            Some(2) if joliet.is_none() => {
                let esc = vd.get(88..91).unwrap_or_default();
                if matches!(esc, b"%/@" | b"%/C" | b"%/E") {
                    let root = parse_record(vd.get(156..190).unwrap_or_default())
                        .ok_or_else(|| bad("root"))?;
                    joliet = Some(Tree { root, joliet: true });
                }
            }
            Some(255) => break,
            _ => {}
        }
    }
    Ok((primary, joliet))
}

/// The records of the directory at `rec` (without `.` and `..`).
fn read_dir(src: &dyn Source, rec: &Record, budget: &mut usize) -> Result<Vec<Record>, FmError> {
    let size = u64::from(rec.size).min(MAX_DIR_BYTES);
    let data = read_at(
        src,
        u64::from(rec.extent) * SECTOR,
        usize::try_from(size).map_err(|_| bad("size"))?,
    )?;
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < data.len() {
        let len = usize::from(data.get(at).copied().unwrap_or(0));
        if len == 0 {
            // The rest of this sector is padding.
            at = (at / SECTOR as usize + 1) * SECTOR as usize;
            continue;
        }
        let end_of_sector = (at / SECTOR as usize + 1) * SECTOR as usize;
        if at + len > end_of_sector {
            return Err(bad("a record crosses a sector"));
        }
        let rec = parse_record(data.get(at..at + len).unwrap_or_default())
            .ok_or_else(|| bad("record"))?;
        *budget = budget
            .checked_sub(1)
            .ok_or_else(|| bad("too many records"))?;
        if !rec.is_dot() {
            out.push(rec);
        }
        at += len;
    }
    Ok(out)
}

/// The system-use bytes Rock Ridge skips in every record, from the root's `SP` entry, or
/// `None` without Rock Ridge.
fn rock_ridge_skip(src: &dyn Source, root: &Record) -> Option<usize> {
    let data = read_at(src, u64::from(root.extent) * SECTOR, 255).ok()?;
    let dot = parse_record(&data)?;
    let su = dot.system_use;
    (su.get(0..2) == Some(b"SP") && su.get(4..6) == Some(&[0xBE, 0xEF]))
        .then(|| usize::from(su.get(6).copied().unwrap_or(0)))
}

pub struct IsoReader {
    src: Arc<dyn Source>,
    index: Index,
    /// Extents `(byte offset, length)` of each file (`Meta::locator` indexes it).
    extents: Vec<Vec<(u64, u64)>>,
    info: Info,
}

impl IsoReader {
    pub fn open(src: Arc<dyn Source>, progress: &Progress<'_>) -> Result<Self, FmError> {
        let (primary, joliet) = volume_trees(src.as_ref())?;
        let primary = primary.ok_or_else(|| bad("no primary volume"))?;
        let rr_skip = rock_ridge_skip(src.as_ref(), &primary.root);
        let (tree, rr, label) = match (rr_skip, joliet) {
            (Some(skip), _) => (primary, Some(skip), "ISO 9660 + Rock Ridge"),
            (None, Some(j)) => (j, None, "ISO 9660 + Joliet"),
            (None, None) => (primary, None, "ISO 9660"),
        };
        let mut index = Index::new();
        let mut extents: Vec<Vec<(u64, u64)>> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut budget = MAX_RECORDS;
        let mut stack: Vec<(Record, String)> = vec![(tree.root.clone(), String::new())];
        seen.insert(tree.root.extent);
        let image_len = src.len();
        while let Some((dir, path)) = stack.pop() {
            if (progress.cancelled)() {
                return Err(FmError::Cancelled);
            }
            let records = read_dir(src.as_ref(), &dir, &mut budget)?;
            let mut pending: Option<Pending> = None;
            for rec in records {
                let facts = rr
                    .map(|skip| rock_ridge(src.as_ref(), &rec.system_use, skip))
                    .unwrap_or_default();
                if facts.relocated {
                    continue;
                }
                let name = facts
                    .name
                    .clone()
                    .unwrap_or_else(|| plain_name(&rec.name, tree.joliet));
                if name.is_empty() {
                    continue;
                }
                let full = if path.is_empty() {
                    name
                } else {
                    format!("{path}/{name}")
                };
                if rec.is_dir() {
                    if seen.insert(rec.extent) {
                        let meta = Meta {
                            modified: rec.modified,
                            mode: facts.mode,
                            ..Meta::default()
                        };
                        index.add(&full, NodeKind::Dir, meta)?;
                        stack.push((rec, full));
                    }
                    continue;
                }
                let start = u64::from(rec.extent) * SECTOR;
                let len = u64::from(rec.size);
                if start.saturating_add(len) > image_len {
                    return Err(bad("a file lies past the end of the image"));
                }
                // Multi-extent: records with the same name continue the file.
                match pending.as_mut() {
                    Some((p, ext, meta, _)) if *p == full => {
                        ext.push((start, len));
                        meta.size = Some(meta.size.unwrap_or(0).saturating_add(len));
                    }
                    _ => {
                        if let Some(done) = pending.take() {
                            add_file(&mut index, &mut extents, done)?;
                        }
                        let kind = facts.link.clone().map_or(NodeKind::File, NodeKind::Symlink);
                        let meta = Meta {
                            size: Some(len),
                            packed: Some(len),
                            modified: rec.modified,
                            mode: facts.mode,
                            ..Meta::default()
                        };
                        pending = Some((full, vec![(start, len)], meta, kind));
                    }
                }
                if !rec.continues()
                    && let Some(done) = pending.take()
                {
                    add_file(&mut index, &mut extents, done)?;
                }
            }
            if let Some(done) = pending.take() {
                add_file(&mut index, &mut extents, done)?;
            }
        }
        Ok(Self {
            src,
            index,
            extents,
            info: Info {
                methods: vec![label.to_string()],
                encrypted: false,
                names_encrypted: false,
            },
        })
    }
}

/// A file being collected: path, extents, metadata, kind (multi-extent files span records).
type Pending = (String, Vec<(u64, u64)>, Meta, NodeKind);

fn add_file(
    index: &mut Index,
    extents: &mut Vec<Vec<(u64, u64)>>,
    (path, ext, mut meta, kind): Pending,
) -> Result<(), FmError> {
    meta.locator = extents.len() as u64;
    index.add(&path, kind, meta)?;
    extents.push(ext);
    Ok(())
}

impl Reader for IsoReader {
    fn format(&self) -> FormatId {
        FormatId::Iso
    }

    fn index(&self) -> &Index {
        &self.index
    }

    fn info(&self) -> Info {
        self.info.clone()
    }

    fn open(&self, id: NodeId, _: &Passwords) -> Result<Box<dyn Read + Send>, FmError> {
        let node = self
            .index
            .node(id)
            .filter(|n| n.kind == NodeKind::File)
            .ok_or_else(|| FmError::NotFound("file".into()))?;
        let ext = usize::try_from(node.meta.locator)
            .ok()
            .and_then(|i| self.extents.get(i))
            .ok_or_else(|| FmError::NotFound("file".into()))?;
        let parts: Vec<Arc<dyn Source>> = ext
            .iter()
            .map(|(at, len)| {
                Arc::new(SliceSource::new(self.src.clone(), *at, *len)) as Arc<dyn Source>
            })
            .collect();
        let joined: Arc<dyn Source> = match parts.as_slice() {
            [one] => one.clone(),
            _ => Arc::new(ConcatSource::new(parts)),
        };
        Ok(Box::new(SourceReader::new(joined)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::source::MemSource;

    /// A directory record with optional system-use bytes.
    fn rec(extent: u32, size: u32, dir: bool, name: &[u8], su: &[u8]) -> Vec<u8> {
        let pad = usize::from(name.len().is_multiple_of(2));
        let len = 33 + name.len() + pad + su.len();
        let mut r = vec![0u8; len];
        r[0] = len as u8;
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[6..10].copy_from_slice(&extent.to_be_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[14..18].copy_from_slice(&size.to_be_bytes());
        r[18..25].copy_from_slice(&[124, 1, 2, 3, 4, 5, 0]);
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r[33 + name.len() + pad..].copy_from_slice(su);
        r
    }

    fn put(img: &mut Vec<u8>, sector: u64, bytes: &[u8]) {
        let at = (sector * SECTOR) as usize;
        if img.len() < at + bytes.len() {
            img.resize(at + bytes.len(), 0);
        }
        img[at..at + bytes.len()].copy_from_slice(bytes);
    }

    fn vd(kind: u8, root: &[u8], joliet: bool) -> Vec<u8> {
        let mut v = vec![0u8; SECTOR as usize];
        v[0] = kind;
        v[1..6].copy_from_slice(b"CD001");
        v[6] = 1;
        v[128..130].copy_from_slice(&2048u16.to_le_bytes());
        if joliet {
            v[88..91].copy_from_slice(b"%/E");
        }
        v[156..156 + root.len()].copy_from_slice(root);
        v
    }

    fn rr_nm(name: &str) -> Vec<u8> {
        let mut e = vec![b'N', b'M', (5 + name.len()) as u8, 1, 0];
        e.extend_from_slice(name.as_bytes());
        e
    }

    /// A small image: plain names (`README.TXT;1`, `DOCS/A.TXT;1`), a Joliet tree with
    /// mixed-case names, and optionally Rock Ridge names, a mode and a symlink.
    pub(crate) fn sample_iso(rock_ridge: bool) -> Vec<u8> {
        let mut img = vec![0u8; (SECTOR * 40) as usize];
        // Primary tree: root at 20, DOCS at 21, files at 30 and 31.
        let sp: Vec<u8> = if rock_ridge {
            vec![b'S', b'P', 7, 1, 0xBE, 0xEF, 0]
        } else {
            vec![]
        };
        let px = |mode: u32| {
            let mut e = vec![b'P', b'X', 44, 1];
            e.extend_from_slice(&mode.to_le_bytes());
            e.extend_from_slice(&mode.to_be_bytes());
            e.extend_from_slice(&[0u8; 32]);
            e
        };
        let su = |name: &str| if rock_ridge { rr_nm(name) } else { vec![] };
        let mut root = rec(20, 2048, true, &[0], &sp);
        root.extend(rec(20, 2048, true, &[1], &[]));
        root.extend(rec(21, 2048, true, b"DOCS", &su("docs")));
        let mut readme_su = su("ReadMe.txt");
        if rock_ridge {
            readme_su.extend(px(0o644));
        }
        root.extend(rec(30, 5, false, b"README.TXT;1", &readme_su));
        if rock_ridge {
            let mut link = rr_nm("latest");
            link.extend_from_slice(&[b'S', b'L', 5 + 2 + 4 + 2 + 5, 1, 0]);
            link.extend_from_slice(&[0, 4, b'd', b'o', b'c', b's', 0, 5]);
            link.extend_from_slice(b"a.txt");
            root.extend(rec(0, 0, false, b"LATEST;1", &link));
        }
        put(&mut img, 20, &root);
        let mut docs = rec(21, 2048, true, &[0], &[]);
        docs.extend(rec(20, 2048, true, &[1], &[]));
        docs.extend(rec(31, 5, false, b"A.TXT;1", &su("a.txt")));
        put(&mut img, 21, &docs);
        put(&mut img, 30, b"hello");
        put(&mut img, 31, b"alpha");
        // Joliet tree: root at 22, Docs at 23.
        let ucs = |s: &str| {
            s.encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<u8>>()
        };
        let mut jroot = rec(22, 2048, true, &[0], &[]);
        jroot.extend(rec(22, 2048, true, &[1], &[]));
        jroot.extend(rec(23, 2048, true, &ucs("Docs"), &[]));
        jroot.extend(rec(30, 5, false, &ucs("Read Me.txt;1"), &[]));
        put(&mut img, 22, &jroot);
        let mut jdocs = rec(23, 2048, true, &[0], &[]);
        jdocs.extend(rec(22, 2048, true, &[1], &[]));
        jdocs.extend(rec(31, 5, false, &ucs("Ä.txt;1"), &[]));
        put(&mut img, 23, &jdocs);
        put(&mut img, 16, &vd(1, &rec(20, 2048, true, &[0], &[]), false));
        put(&mut img, 17, &vd(2, &rec(22, 2048, true, &[0], &[]), true));
        let mut term = vec![0u8; SECTOR as usize];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        put(&mut img, 18, &term);
        img
    }

    fn open(img: Vec<u8>) -> IsoReader {
        IsoReader::open(Arc::new(MemSource::new(img)), &Progress::none()).expect("iso")
    }

    fn read(r: &IsoReader, path: &str) -> String {
        let id = r.index().lookup(path).expect(path);
        let mut s = String::new();
        r.open(id, &Passwords::default())
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        s
    }

    #[test]
    fn rock_ridge_names_modes_and_links_win() {
        let r = open(sample_iso(true));
        assert_eq!(read(&r, "ReadMe.txt"), "hello");
        assert_eq!(read(&r, "docs/a.txt"), "alpha");
        let id = r.index().lookup("ReadMe.txt").expect("readme");
        assert_eq!(r.index().node(id).and_then(|n| n.meta.mode), Some(0o644));
        let link = r.index().lookup("latest").expect("link");
        assert_eq!(
            r.index().node(link).map(|n| n.kind.clone()),
            Some(NodeKind::Symlink("docs/a.txt".into()))
        );
        assert_eq!(r.info().methods, vec!["ISO 9660 + Rock Ridge"]);
    }

    #[test]
    fn joliet_names_are_used_without_rock_ridge() {
        let r = open(sample_iso(false));
        assert_eq!(read(&r, "Read Me.txt"), "hello");
        assert_eq!(read(&r, "Docs/Ä.txt"), "alpha");
        assert_eq!(r.info().methods, vec!["ISO 9660 + Joliet"]);
    }

    #[test]
    fn plain_names_lose_their_version() {
        assert_eq!(plain_name(b"README.TXT;1", false), "README.TXT");
        assert_eq!(plain_name(b"DIR.", false), "DIR");
        assert_eq!(plain_name(b"A;B", false), "A;B", "not a version number");
    }

    /// Byte offset of record `n` (0 = `.`) in `sector`.
    fn record_at(img: &[u8], sector: u64, n: usize) -> usize {
        let mut at = (sector * SECTOR) as usize;
        for _ in 0..n {
            at += usize::from(img[at]);
        }
        at
    }

    #[test]
    fn a_directory_loop_and_out_of_range_extents_are_safe() {
        // Joliet "Docs" points back at the Joliet root: walked once, no endless loop.
        let mut img = sample_iso(false);
        let docs = record_at(&img, 22, 2);
        img[docs + 2..docs + 6].copy_from_slice(&22u32.to_le_bytes());
        let r = open(img);
        assert_eq!(read(&r, "Read Me.txt"), "hello");
        // A file extent past the end is an error, not a panic or a huge read.
        let mut img = sample_iso(false);
        let file = record_at(&img, 23, 2);
        img[file + 2..file + 6].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
        assert!(IsoReader::open(Arc::new(MemSource::new(img)), &Progress::none()).is_err());
        // A record whose name runs past its own length is refused.
        let mut img = sample_iso(false);
        let at = record_at(&img, 22, 2);
        img[at + 32] = 200;
        assert!(IsoReader::open(Arc::new(MemSource::new(img)), &Progress::none()).is_err());
    }
}
