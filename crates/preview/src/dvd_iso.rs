//! DVD-Video disc images (`.iso`): a minimal, bounded ISO-9660 reader that finds the
//! main title's `VIDEO_TS/VTS_nn_m.VOB` parts. DVD-Video stores a title set's parts
//! back to back, so the whole film is one byte range that ffmpeg reads through its
//! `subfile` protocol. No mounting, no extraction, no temporary files. Call on a worker.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use fm_vfs::FmError;

const SECTOR: u64 = 2048;
/// Volume descriptors start at sector 16; real images have a handful.
const MAX_DESCRIPTORS: u64 = 32;
/// Directories larger than this are refused (VIDEO_TS holds a few dozen files).
const MAX_DIR_BYTES: u32 = 1 << 20;

/// The film inside a disc image: bytes `start..end` hold title set `title`'s parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DvdTitle {
    pub title: u8,
    pub parts: u8,
    pub start: u64,
    pub end: u64,
}

fn invalid(message: &str) -> FmError {
    FmError::backend("DVD image", message)
}

#[derive(Debug)]
struct Record {
    name: String,
    lba: u32,
    size: u32,
    dir: bool,
}

fn read_at(src: &mut (impl Read + Seek), offset: u64, buf: &mut [u8]) -> Result<(), FmError> {
    src.seek(SeekFrom::Start(offset)).map_err(FmError::Io)?;
    src.read_exact(buf)
        .map_err(|_| invalid("The disc image is truncated."))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let b: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(u32::from_le_bytes(b))
}

/// Parse one directory record at the start of `bytes`; returns it and its length.
fn record(bytes: &[u8]) -> Option<(Record, usize)> {
    let len = usize::from(*bytes.first()?);
    if len < 34 || len > bytes.len() {
        return None;
    }
    let lba = le32(bytes, 2)?;
    let size = le32(bytes, 10)?;
    let flags = *bytes.get(25)?;
    // Multi-extent records (bit 7) are not used by DVD-Video and are skipped.
    if flags & 0x80 != 0 {
        return None;
    }
    let name_len = usize::from(*bytes.get(32)?);
    let raw = bytes.get(33..33 + name_len)?;
    let name = String::from_utf8_lossy(raw);
    let name = name
        .split(';')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    Some((
        Record {
            name,
            lba,
            size,
            dir: flags & 0x02 != 0,
        },
        len,
    ))
}

fn read_dir(src: &mut (impl Read + Seek), lba: u32, size: u32) -> Result<Vec<Record>, FmError> {
    if size > MAX_DIR_BYTES {
        return Err(invalid("A folder in the disc image is too large."));
    }
    let mut data = vec![0u8; size as usize];
    read_at(src, u64::from(lba) * SECTOR, &mut data)?;
    let mut records = Vec::new();
    for sector in data.chunks(SECTOR as usize) {
        // Records never cross a sector; a zero length byte pads to the next one.
        let mut at = 0;
        while let Some(rest) = sector.get(at..) {
            match rest.first() {
                None | Some(0) => break,
                Some(&len) => {
                    if let Some((record, _)) = record(rest) {
                        records.push(record);
                    }
                    at += usize::from(len).max(1);
                }
            }
        }
    }
    Ok(records)
}

/// The largest title set of a DVD-Video image, as one contiguous byte range.
pub fn main_title(src: &mut (impl Read + Seek), image_len: u64) -> Result<DvdTitle, FmError> {
    let mut root = None;
    for index in 16..16 + MAX_DESCRIPTORS {
        let mut descriptor = [0u8; 2048];
        read_at(src, index * SECTOR, &mut descriptor)?;
        if descriptor.get(1..6) != Some(b"CD001") {
            return Err(invalid("This is not an ISO-9660 disc image."));
        }
        match descriptor.first() {
            Some(1) => {
                root = descriptor.get(156..190).and_then(record).map(|(r, _)| r);
                break;
            }
            Some(255) => break,
            _ => {}
        }
    }
    let root = root.ok_or_else(|| invalid("This disc image has no readable file table."))?;
    let video_ts = read_dir(src, root.lba, root.size)?
        .into_iter()
        .find(|r| r.dir && r.name == "VIDEO_TS")
        .ok_or_else(|| invalid("This disc image is not a DVD-Video disc."))?;
    let mut parts: Vec<(u8, u8, u64, u64)> = read_dir(src, video_ts.lba, video_ts.size)?
        .into_iter()
        .filter(|r| !r.dir)
        .filter_map(|r| match fm_media::dvd_file(&r.name)? {
            fm_media::DvdFile::TitlePart { title, part } => {
                let start = u64::from(r.lba) * SECTOR;
                Some((title, part, start, start + u64::from(r.size)))
            }
            fm_media::DvdFile::Menu => None,
        })
        .collect();
    parts.sort_unstable();
    let mut best: Option<DvdTitle> = None;
    for title in 1..=99u8 {
        // Parts 1, 2, 3… of this title set, as long as each follows the previous on disc.
        let mut run: Option<DvdTitle> = None;
        for &(_, part, start, end) in parts.iter().filter(|p| p.0 == title) {
            match &mut run {
                None if part == 1 => {
                    run = Some(DvdTitle {
                        title,
                        parts: 1,
                        start,
                        end,
                    })
                }
                Some(r) if part == r.parts + 1 && start == r.end.div_ceil(SECTOR) * SECTOR => {
                    r.parts = part;
                    r.end = end;
                }
                _ => break,
            }
        }
        if let Some(run) = run
            && run.end <= image_len
            && run.end > run.start
            && best.is_none_or(|b| run.end - run.start > b.end - b.start)
        {
            best = Some(run);
        }
    }
    best.ok_or_else(|| invalid("This DVD image contains no playable title."))
}

/// [`main_title`] of the image file at `path`.
pub fn main_title_of(path: &Path) -> Result<DvdTitle, FmError> {
    let mut file = std::fs::File::open(path).map_err(FmError::Io)?;
    let len = file.metadata().map_err(FmError::Io)?.len();
    main_title(&mut file, len)
}

/// Whether `path` is a DVD-Video image (cheap: reads a few sectors).
pub fn is_dvd_image(path: &Path) -> bool {
    main_title_of(path).is_ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Cursor;

    fn dir_record(name: &str, lba: u32, size: u32, dir: bool) -> Vec<u8> {
        let name = name.as_bytes();
        let mut len = 33 + name.len();
        if len % 2 == 1 {
            len += 1;
        }
        let mut r = vec![0u8; len];
        r[0] = len as u8;
        r[2..6].copy_from_slice(&lba.to_le_bytes());
        r[6..10].copy_from_slice(&lba.to_be_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[14..18].copy_from_slice(&size.to_be_bytes());
        r[25] = if dir { 2 } else { 0 };
        r[32] = name.len() as u8;
        r[33..33 + name.len()].copy_from_slice(name);
        r
    }

    /// A tiny DVD-Video ISO: `files` are (name, lba, size) inside VIDEO_TS.
    pub(crate) fn image(files: &[(&str, u32, u32)], sectors: u32) -> Vec<u8> {
        let mut iso = vec![0u8; sectors as usize * 2048];
        let put = |iso: &mut Vec<u8>, lba: u32, bytes: &[u8]| {
            let at = lba as usize * 2048;
            iso[at..at + bytes.len()].copy_from_slice(bytes);
        };
        let mut pvd = vec![0u8; 2048];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        let root = dir_record("\0", 18, 2048, true);
        pvd[156..156 + root.len()].copy_from_slice(&root);
        put(&mut iso, 16, &pvd);
        let mut end = vec![0u8; 2048];
        end[0] = 255;
        end[1..6].copy_from_slice(b"CD001");
        put(&mut iso, 17, &end);
        let mut root_dir = dir_record("\0", 18, 2048, true);
        root_dir.extend(dir_record("\u{1}", 18, 2048, true));
        root_dir.extend(dir_record("AUDIO_TS", 20, 2048, true));
        root_dir.extend(dir_record("VIDEO_TS", 19, 2048, true));
        put(&mut iso, 18, &root_dir);
        let mut video = Vec::new();
        for (name, lba, size) in files {
            video.extend(dir_record(&format!("{name};1"), *lba, *size, false));
        }
        put(&mut iso, 19, &video);
        iso
    }

    #[test]
    fn the_largest_contiguous_title_set_is_the_film() {
        let iso = image(
            &[
                ("VIDEO_TS.VOB", 30, 2048),
                ("VTS_01_0.VOB", 31, 2048),
                ("VTS_01_1.VOB", 32, 4096),
                ("VTS_02_0.VOB", 40, 2048),
                ("VTS_02_1.VOB", 41, 8192),
                ("VTS_02_2.VOB", 45, 6000),
            ],
            60,
        );
        let len = iso.len() as u64;
        let title = main_title(&mut Cursor::new(iso), len).expect("title");
        assert_eq!(
            title,
            DvdTitle {
                title: 2,
                parts: 2,
                start: 41 * 2048,
                end: 45 * 2048 + 6000
            }
        );
    }

    #[test]
    fn a_real_film_inside_an_image_probes_and_decodes_across_parts() {
        use crate::video::find_tool;
        let (Some(ffmpeg), Some(_)) = (find_tool("ffmpeg"), find_tool("ffprobe")) else {
            return;
        };
        let dir = tempfile::tempdir().expect("tmp");
        let vob = dir.path().join("film.vob");
        let made = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-f", "lavfi", "-i"])
            .arg("testsrc=size=720x576:rate=25:duration=4")
            .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=4"])
            .args(["-c:v", "mpeg2video", "-c:a", "mp2", "-f", "vob"])
            .arg(&vob)
            .status();
        if !made.is_ok_and(|s| s.success()) {
            return; // ffmpeg without the MPEG-2 encoder
        }
        let film = std::fs::read(&vob).expect("vob");
        // VOB packs are 2048 bytes, so a pack boundary is a clean place to split parts.
        let half = (film.len() / 2) / 2048 * 2048;
        let (one, two) = film.split_at(half);
        let first = 30u32;
        let second = first + (one.len() as u32).div_ceil(2048);
        let sectors = second + (two.len() as u32).div_ceil(2048) + 2;
        let mut iso = image(
            &[
                ("VIDEO_TS.VOB", 25, 2048),
                ("VTS_01_1.VOB", first, one.len() as u32),
                ("VTS_01_2.VOB", second, two.len() as u32),
            ],
            sectors,
        );
        iso[first as usize * 2048..][..one.len()].copy_from_slice(one);
        iso[second as usize * 2048..][..two.len()].copy_from_slice(two);
        let path = dir.path().join("Movie.iso");
        std::fs::write(&path, &iso).expect("iso");

        let title = main_title_of(&path).expect("title");
        assert_eq!(title.parts, 2);
        let uri = fm_vfs::VfsUri::parse(&path.to_string_lossy()).expect("uri");
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let info = crate::video::playback_info(&uri, &cancelled).expect("probe");
        assert_eq!((info.width, info.height), (720, 576));
        assert!(info.audio.is_some());
        assert_eq!(info.range, Some((title.start, title.end)));
        assert!(info.duration > 3.5, "both parts count: {}", info.duration);
        // Decode every frame through the same input the player uses: both parts play.
        let (protocols, input) = info.ffmpeg_input().expect("input");
        let out = std::process::Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-nostdin",
                "-protocol_whitelist",
                protocols,
                "-i",
            ])
            .arg(input)
            .args(["-map", "0:v:0", "-f", "null", "-"])
            .args(["-stats_period", "100"])
            .output()
            .expect("decode");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // A data image is not playable.
        let data = dir.path().join("data.iso");
        std::fs::write(&data, vec![0u8; 40 * 2048]).expect("data");
        assert!(!is_dvd_image(&data));
        let uri = fm_vfs::VfsUri::parse(&data.to_string_lossy()).expect("uri");
        assert!(crate::video::playback_info(&uri, &cancelled).is_err());
    }

    #[test]
    fn a_gap_between_parts_ends_the_range() {
        let iso = image(
            &[("VTS_01_1.VOB", 30, 4096), ("VTS_01_2.VOB", 40, 4096)],
            50,
        );
        let len = iso.len() as u64;
        let title = main_title(&mut Cursor::new(iso), len).expect("title");
        assert_eq!((title.parts, title.end), (1, 30 * 2048 + 4096));
    }

    #[test]
    fn hostile_or_foreign_images_are_errors() {
        let len = |v: &Vec<u8>| v.len() as u64;
        // Not ISO-9660 at all.
        let junk = vec![0x5a; 40 * 2048];
        assert!(main_title(&mut Cursor::new(junk.clone()), len(&junk)).is_err());
        // Too short to hold descriptors.
        let short = vec![0u8; 100];
        assert!(main_title(&mut Cursor::new(short.clone()), len(&short)).is_err());
        // An ISO without VIDEO_TS (a data disc).
        let mut data = image(&[], 30);
        let at = 18 * 2048;
        for b in &mut data[at..at + 2048] {
            *b = 0;
        }
        assert!(main_title(&mut Cursor::new(data.clone()), len(&data)).is_err());
        // Menus only.
        let menus = image(&[("VIDEO_TS.VOB", 25, 2048)], 30);
        assert!(main_title(&mut Cursor::new(menus.clone()), len(&menus)).is_err());
        // A part pointing past the end of the image.
        let past = image(&[("VTS_01_1.VOB", 25, u32::MAX)], 30);
        assert!(main_title(&mut Cursor::new(past.clone()), len(&past)).is_err());
        // A huge directory size is refused before allocating.
        let mut big = image(&[("VTS_01_1.VOB", 25, 2048)], 30);
        let root = 16 * 2048 + 156;
        big[root + 10..root + 14].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(main_title(&mut Cursor::new(big.clone()), len(&big)).is_err());
        // Garbage directory records never panic.
        let mut garbage = image(&[("VTS_01_1.VOB", 25, 2048)], 30);
        for (i, b) in garbage[19 * 2048..20 * 2048].iter_mut().enumerate() {
            *b = (i * 37 % 251) as u8;
        }
        let _ = main_title(&mut Cursor::new(garbage.clone()), len(&garbage));
    }
}
