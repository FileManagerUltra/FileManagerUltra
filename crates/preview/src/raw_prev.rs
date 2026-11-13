//! Camera RAW photos (DNG, CR2, NEF, ARW, ORF, PEF…): shown through the JPEG preview the
//! camera or converter embedded in the TIFF structure, the way most file managers do. The
//! sensor data itself is never decoded.
//!
//! The file is hostile input: every offset is bounds-checked, directories are visited at most
//! once and counted, and only small header reads happen before the chosen JPEG is read.

use std::collections::HashSet;
use std::io::Read;

use fm_vfs::{Entry, FmError, Vfs};

use crate::{Limits, MetaDoc, Preview, base_meta};

/// Directories visited per file, entries read per directory and SubIFD nesting.
const MAX_IFDS: usize = 32;
const MAX_ENTRIES: usize = 1024;
const MAX_DEPTH: u8 = 3;
/// SubIFD offsets followed per directory.
const MAX_SUBIFDS: usize = 16;
/// Bytes read from the start of a candidate JPEG to find its frame header.
const JPEG_HEAD: usize = 64 << 10;

#[cfg(test)]
const TAG_SUBFILE_TYPE: u16 = 254;
#[cfg(test)]
const TAG_WIDTH: u16 = 256;
const TAG_COMPRESSION: u16 = 259;
const TAG_PHOTOMETRIC: u16 = 262;
const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_ORIENTATION: u16 = 274;
const TAG_STRIP_BYTES: u16 = 279;
const TAG_SUBIFDS: u16 = 330;
const TAG_JPEG_OFFSET: u16 = 513;
const TAG_JPEG_LENGTH: u16 = 514;
/// Compression: old-style and new-style JPEG.
const JPEG_COMPRESSION: [u32; 2] = [6, 7];
/// Photometric interpretations of sensor data, never a picture to show.
const SENSOR_DATA: [u32; 2] = [32803, 34892];

/// File name extensions of TIFF-based camera RAW formats.
pub const RAW_EXTENSIONS: [&str; 12] = [
    "dng", "cr2", "nef", "nrw", "arw", "sr2", "srf", "orf", "pef", "srw", "erf", "3fr",
];

/// Whether `name` has a camera RAW extension.
pub fn is_raw_name(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| {
        RAW_EXTENSIONS
            .iter()
            .any(|raw| ext.eq_ignore_ascii_case(raw))
    })
}

/// Whether `head` starts like a TIFF structure: classic TIFF and the Olympus ORF variants.
pub fn is_tiff(head: &[u8]) -> bool {
    matches!(
        head.get(..4),
        Some(b"II*\0" | b"MM\0*" | b"IIRO" | b"IIRS" | b"MMOR")
    )
}

/// Random access to the file being read.
trait Source {
    /// Exactly `len` bytes at `offset`, or `None` when they are not there.
    fn read(&mut self, offset: u64, len: usize) -> Option<Vec<u8>>;
}

impl Source for &[u8] {
    fn read(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        let start = usize::try_from(offset).ok()?;
        self.get(start..start.checked_add(len)?).map(<[u8]>::to_vec)
    }
}

struct VfsSource<'a> {
    vfs: &'a dyn Vfs,
    entry: &'a Entry,
}

impl Source for VfsSource<'_> {
    fn read(&mut self, offset: u64, len: usize) -> Option<Vec<u8>> {
        if self
            .entry
            .size
            .is_some_and(|size| offset.saturating_add(len as u64) > size)
        {
            return None;
        }
        let reader = self.vfs.open_read_at(&self.entry.uri, offset).ok()?;
        let mut out = Vec::with_capacity(len.min(1 << 20));
        reader.take(len as u64).read_to_end(&mut out).ok()?;
        (out.len() == len).then_some(out)
    }
}

#[derive(Clone, Copy)]
struct Order {
    little: bool,
}

impl Order {
    fn u16(self, b: &[u8], at: usize) -> Option<u16> {
        let b: [u8; 2] = b.get(at..at + 2)?.try_into().ok()?;
        Some(if self.little {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        })
    }

    fn u32(self, b: &[u8], at: usize) -> Option<u32> {
        let b: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        Some(if self.little {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        })
    }
}

/// One directory entry: its type, count and the raw 4-byte value/offset field.
#[derive(Clone, Copy)]
struct Field {
    kind: u16,
    count: u32,
    value: [u8; 4],
}

impl Field {
    /// The first value of a SHORT or LONG field.
    fn first(self, order: Order) -> Option<u32> {
        match self.kind {
            3 => order.u16(&self.value, 0).map(u32::from),
            4 | 13 => order.u32(&self.value, 0),
            _ => None,
        }
    }
}

/// A JPEG stored inside the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Embedded {
    offset: u64,
    len: u64,
}

/// What the TIFF structure offers: candidate JPEGs and the orientation of the photo.
struct Found {
    jpegs: Vec<Embedded>,
    orientation: u16,
}

fn walk(source: &mut dyn Source) -> Option<Found> {
    let header = source.read(0, 8)?;
    if !is_tiff(&header) {
        return None;
    }
    let order = Order {
        little: header.first() == Some(&b'I'),
    };
    let mut queue = vec![(u64::from(order.u32(&header, 4)?), 0u8, true)];
    let mut seen = HashSet::new();
    let mut found = Found {
        jpegs: Vec::new(),
        orientation: 1,
    };
    while let Some((offset, depth, top)) = queue.pop() {
        if offset < 8 || !seen.insert(offset) || seen.len() > MAX_IFDS {
            continue;
        }
        let Some(count) = source.read(offset, 2).and_then(|b| order.u16(&b, 0)) else {
            continue;
        };
        let count = usize::from(count).min(MAX_ENTRIES);
        let Some(table) = source.read(offset + 2, count * 12 + 4) else {
            continue;
        };
        let mut fields = std::collections::HashMap::new();
        for i in 0..count {
            let at = i * 12;
            let (Some(tag), Some(kind), Some(n), Some(value)) = (
                order.u16(&table, at),
                order.u16(&table, at + 2),
                order.u32(&table, at + 4),
                table
                    .get(at + 8..at + 12)
                    .and_then(|v| <[u8; 4]>::try_from(v).ok()),
            ) else {
                break;
            };
            fields.insert(
                tag,
                Field {
                    kind,
                    count: n,
                    value,
                },
            );
        }
        let get = |tag: u16| fields.get(&tag).and_then(|f| f.first(order));
        if top && depth == 0 && found.orientation == 1 {
            found.orientation = get(TAG_ORIENTATION)
                .and_then(|o| u16::try_from(o).ok())
                .filter(|o| (1..=8).contains(o))
                .unwrap_or(1);
        }
        let sensor = get(TAG_PHOTOMETRIC).is_some_and(|p| SENSOR_DATA.contains(&p));
        let jpeg = get(TAG_COMPRESSION).is_some_and(|c| JPEG_COMPRESSION.contains(&c));
        // One strip holding a whole JPEG (CR2's full-size preview, DNG previews).
        let single_strip = |tag: u16| {
            fields
                .get(&tag)
                .filter(|f| f.count == 1)
                .and_then(|f| f.first(order))
        };
        if jpeg
            && !sensor
            && let (Some(offset), Some(len)) = (
                single_strip(TAG_STRIP_OFFSETS),
                single_strip(TAG_STRIP_BYTES),
            )
        {
            found.jpegs.push(Embedded {
                offset: u64::from(offset),
                len: u64::from(len),
            });
        }
        // The old JPEGInterchangeFormat pair (thumbnails in IFD1, NEF previews).
        if !sensor && let (Some(offset), Some(len)) = (get(TAG_JPEG_OFFSET), get(TAG_JPEG_LENGTH)) {
            found.jpegs.push(Embedded {
                offset: u64::from(offset),
                len: u64::from(len),
            });
        }
        if depth < MAX_DEPTH
            && let Some(sub) = fields.get(&TAG_SUBIFDS)
            && matches!(sub.kind, 4 | 13)
        {
            let n = (sub.count as usize).min(MAX_SUBIFDS);
            let offsets = if n <= 1 {
                Some(sub.value.to_vec())
            } else {
                order
                    .u32(&sub.value, 0)
                    .and_then(|at| source.read(u64::from(at), n * 4))
            };
            for i in 0..n {
                if let Some(at) = offsets.as_deref().and_then(|b| order.u32(b, i * 4)) {
                    queue.push((u64::from(at), depth + 1, false));
                }
            }
        }
        if let Some(next) = order.u32(&table, count * 12).filter(|n| *n != 0) {
            queue.push((u64::from(next), depth, top));
        }
    }
    Some(found)
}

/// Width and height from a baseline or progressive JPEG frame header; `None` for lossless
/// (sensor data) and anything that is not a JPEG the decoder can read.
fn jpeg_size(head: &[u8]) -> Option<(u32, u32)> {
    if head.get(..2) != Some(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    loop {
        if *head.get(at)? != 0xFF {
            return None;
        }
        let marker = *head.get(at + 1)?;
        if marker == 0xFF {
            at += 1;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*head.get(at + 2)?, *head.get(at + 3)?]));
        match marker {
            // Baseline, extended sequential and progressive Huffman frames.
            0xC0..=0xC2 => {
                let h = u16::from_be_bytes([*head.get(at + 5)?, *head.get(at + 6)?]);
                let w = u16::from_be_bytes([*head.get(at + 7)?, *head.get(at + 8)?]);
                return (w > 0 && h > 0).then_some((u32::from(w), u32::from(h)));
            }
            // Other frame types (lossless, arithmetic) or the scan itself: not for us.
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDA => return None,
            _ => at += 2 + len,
        }
    }
}

/// The largest embedded JPEG the decoder can show, read in full, with the photo's orientation.
fn best_jpeg(source: &mut dyn Source, limits: &Limits) -> Option<(Vec<u8>, u16)> {
    let found = walk(source)?;
    let mut best: Option<(u64, Embedded)> = None;
    for jpeg in found.jpegs.iter().take(MAX_IFDS) {
        if jpeg.len < 4 || jpeg.len > limits.image_bytes {
            continue;
        }
        let head_len = usize::try_from(jpeg.len).ok()?.min(JPEG_HEAD);
        let Some((w, h)) = source
            .read(jpeg.offset, head_len)
            .as_deref()
            .and_then(jpeg_size)
        else {
            continue;
        };
        let area = u64::from(w) * u64::from(h);
        if area <= limits.image_pixels && best.is_none_or(|(a, _)| area > a) {
            best = Some((area, *jpeg));
        }
    }
    let (_, jpeg) = best?;
    let bytes = source.read(jpeg.offset, usize::try_from(jpeg.len).ok()?)?;
    Some((bytes, found.orientation))
}

/// Label for the info line, from the extension.
fn label(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("dng") => "DNG camera preview",
        Some("cr2") => "Canon CR2 camera preview",
        Some("nef" | "nrw") => "Nikon NEF camera preview",
        Some("arw" | "sr2" | "srf") => "Sony ARW camera preview",
        Some("orf") => "Olympus ORF camera preview",
        Some("pef") => "Pentax PEF camera preview",
        Some("srw") => "Samsung SRW camera preview",
        _ => "RAW camera preview",
    }
}

fn no_preview(entry: &Entry) -> Preview {
    let mut rows = base_meta(entry);
    rows.push((
        "Image".into(),
        "no embedded picture that can be shown".into(),
    ));
    Preview::Meta(MetaDoc {
        rows,
        hint: Some(
            "RAW photos and TIFF files are shown through the JPEG preview stored inside them; \
             this file has none."
                .into(),
        ),
        poster: None,
        video: None,
        audio: None,
    })
}

fn show(entry: &Entry, found: Option<(Vec<u8>, u16)>, limits: &Limits) -> Preview {
    match found {
        Some((jpeg, orientation)) => crate::image_prev::decode_embedded(
            entry,
            &jpeg,
            limits,
            orientation,
            label(&entry.name),
        ),
        None => no_preview(entry),
    }
}

/// Preview a RAW photo from the file, reading only its directories and the chosen JPEG.
pub fn render(vfs: &dyn Vfs, entry: &Entry, limits: &Limits) -> Result<Preview, FmError> {
    let mut source = VfsSource { vfs, entry };
    Ok(show(entry, best_jpeg(&mut source, limits), limits))
}

/// Preview RAW bytes already in memory (thumbnails, Copy Image, Markdown images).
pub fn decode(entry: &Entry, bytes: &[u8], limits: &Limits) -> Preview {
    let mut source = bytes;
    show(entry, best_jpeg(&mut source, limits), limits)
}

/// The embedded JPEG of RAW bytes and the photo's EXIF orientation, for full-size copies.
pub(crate) fn embedded_jpeg(bytes: &[u8]) -> Option<(Vec<u8>, u16)> {
    let mut source = bytes;
    best_jpeg(&mut source, &Limits::default())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tiny baseline JPEG of `w`×`h`, red.
    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(w, h, image::Rgb([220, 30, 30]));
        let mut out = Vec::new();
        img.write_to(
            &mut std::io::Cursor::new(&mut out),
            image::ImageFormat::Jpeg,
        )
        .expect("jpeg");
        out
    }

    /// Little-endian TIFF: IFD0 (orientation, optional strip JPEG, SubIFDs), each SubIFD
    /// one more strip JPEG, plus a lossless "sensor" strip that must never be picked.
    pub(crate) fn raw_file(orientation: u16, previews: &[(u32, u32)]) -> Vec<u8> {
        let mut data: Vec<u8> = b"II*\0\0\0\0\0".to_vec();
        let entry = |tag: u16, kind: u16, count: u32, value: u32| {
            let mut e = Vec::new();
            e.extend_from_slice(&tag.to_le_bytes());
            e.extend_from_slice(&kind.to_le_bytes());
            e.extend_from_slice(&count.to_le_bytes());
            e.extend_from_slice(&value.to_le_bytes());
            e
        };
        let ifd = |data: &mut Vec<u8>, entries: Vec<Vec<u8>>| -> u32 {
            let at = data.len() as u32;
            data.extend_from_slice(&(entries.len() as u16).to_le_bytes());
            for e in entries {
                data.extend_from_slice(&e);
            }
            data.extend_from_slice(&0u32.to_le_bytes());
            at
        };
        // Sensor data: a lossless JPEG header (SOF3) that is larger than any preview.
        let sensor_at = data.len() as u32;
        let sensor = [
            0xFF, 0xD8, 0xFF, 0xC3, 0, 11, 8, 0x01, 0, 0x02, 0, 1, 1, 0x11, 0,
        ];
        data.extend_from_slice(&sensor);
        let mut subs = Vec::new();
        for &(w, h) in previews {
            let bytes = jpeg(w, h);
            let at = data.len() as u32;
            data.extend_from_slice(&bytes);
            subs.push(ifd(
                &mut data,
                vec![
                    entry(TAG_SUBFILE_TYPE, 4, 1, 1),
                    entry(TAG_WIDTH, 4, 1, w),
                    entry(TAG_COMPRESSION, 3, 1, 7),
                    entry(TAG_PHOTOMETRIC, 3, 1, 6),
                    entry(TAG_STRIP_OFFSETS, 4, 1, at),
                    entry(TAG_STRIP_BYTES, 4, 1, bytes.len() as u32),
                ],
            ));
        }
        subs.push(ifd(
            &mut data,
            vec![
                entry(TAG_COMPRESSION, 3, 1, 6),
                entry(TAG_STRIP_OFFSETS, 4, 1, sensor_at),
                entry(TAG_STRIP_BYTES, 4, 1, sensor.len() as u32),
            ],
        ));
        let list_at = data.len() as u32;
        for sub in &subs {
            data.extend_from_slice(&sub.to_le_bytes());
        }
        let ifd0 = ifd(
            &mut data,
            vec![
                entry(TAG_ORIENTATION, 3, 1, u32::from(orientation)),
                entry(TAG_SUBIFDS, 4, subs.len() as u32, list_at),
            ],
        );
        data.splice(4..8, ifd0.to_le_bytes());
        data
    }

    fn entry(name: &str) -> Entry {
        Entry::new(
            fm_vfs::VfsUri::parse(&format!("/photos/{name}")).expect("uri"),
            fm_vfs::EntryKind::File,
        )
    }

    #[test]
    fn the_largest_embedded_jpeg_is_shown_and_sensor_data_is_skipped() {
        let raw = raw_file(1, &[(32, 20), (96, 64), (48, 32)]);
        let Preview::Image(doc) = decode(&entry("a.dng"), &raw, &Limits::default()) else {
            panic!("no picture");
        };
        assert_eq!((doc.width, doc.height), (96, 64));
        assert_eq!(doc.format, "DNG camera preview");
    }

    #[test]
    fn the_cameras_orientation_turns_the_preview() {
        let raw = raw_file(6, &[(96, 64)]);
        let Preview::Image(doc) = decode(&entry("a.cr2"), &raw, &Limits::default()) else {
            panic!("no picture");
        };
        assert_eq!((doc.width, doc.height, doc.original), (64, 96, (64, 96)));
    }

    #[test]
    fn reading_from_a_file_matches_reading_from_memory() {
        let vfs = fm_testkit::MemFs::new();
        vfs.mkdirs_str("/photos").expect("dir");
        vfs.put_str("/photos/b.nef", &raw_file(1, &[(40, 30), (80, 60)]))
            .expect("write");
        let uri = fm_vfs::VfsUri::parse("/photos/b.nef").expect("uri");
        let entry = vfs.stat(&uri).expect("stat");
        let Ok(Preview::Image(doc)) = render(&vfs, &entry, &Limits::default()) else {
            panic!("no picture");
        };
        assert_eq!((doc.width, doc.height), (80, 60));
    }

    #[test]
    fn hostile_and_preview_less_files_never_panic() {
        let limits = Limits::default();
        assert!(matches!(
            decode(&entry("x.dng"), &raw_file(1, &[]), &limits),
            Preview::Meta(_)
        ));
        let good = raw_file(1, &[(32, 20)]);
        // Every truncation and a byte flipped at every position: no panic, no hang.
        for cut in 0..good.len() {
            let _ = decode(
                &entry("x.dng"),
                good.get(..cut).unwrap_or_default(),
                &limits,
            );
        }
        for at in 0..good.len().min(400) {
            let mut bad = good.clone();
            if let Some(b) = bad.get_mut(at) {
                *b ^= 0xFF;
            }
            let _ = decode(&entry("x.dng"), &bad, &limits);
        }
        // An IFD that points at itself, and a huge entry count.
        let mut looped =
            b"II*\0\x08\0\0\0\x01\0\x4a\x01\x04\0\x01\0\0\0\x08\0\0\0\x08\0\0\0".to_vec();
        assert!(matches!(
            decode(&entry("x.dng"), &looped, &limits),
            Preview::Meta(_)
        ));
        looped.splice(8..10, [0xFF, 0xFF]);
        assert!(matches!(
            decode(&entry("x.dng"), &looped, &limits),
            Preview::Meta(_)
        ));
        assert!(!is_tiff(b"II") && is_tiff(b"MM\0*") && is_tiff(b"IIRO"));
        assert!(is_raw_name("IMG_1.CR2") && is_raw_name("a.dng") && !is_raw_name("a.tif"));
    }

    #[test]
    fn jpeg_frames_are_read_and_lossless_ones_refused() {
        assert_eq!(jpeg_size(&jpeg(30, 20)), Some((30, 20)));
        assert_eq!(
            jpeg_size(&[0xFF, 0xD8, 0xFF, 0xC3, 0, 11, 8, 0, 4, 0, 4, 1, 1, 0x11, 0]),
            None
        );
        assert_eq!(jpeg_size(b"nope"), None);
        assert_eq!(jpeg_size(&[0xFF, 0xD8, 0xFF]), None);
    }
}
