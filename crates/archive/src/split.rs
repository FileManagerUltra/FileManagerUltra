//! Split archives (design D16): numbered parts (`name.ext.001`, …) of any format and zip
//! spanned sets (`name.z01`, …, `name.zip`). Sets are found by name only, in one folder
//! listing, capped at [`MAX_PARTS`]; a gap fails with a message naming the missing part.
//! Numbered parts are read as one file through a [`ConcatSource`]; a spanned zip is
//! presented as one ordinary zip by rewriting its central directory's per-disk offsets
//! into offsets in the joined bytes.

use std::sync::Arc;

use fm_vfs::FmError;

use crate::format::{FormatId, by_name};
use crate::source::{ConcatSource, MemSource, Source};

/// Most parts one split archive may have.
pub const MAX_PARTS: u32 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitKind {
    /// `backup.7z.001`, `.002`, …: the archive's bytes cut at fixed sizes.
    Numbered,
    /// `site.z01`, `site.z02`, …, `site.zip` (last): a zip with per-disk offsets.
    ZipSpanned,
    /// `data.part1.rar`, `data.part2.rar`, … or `data.rar`, `data.r00`, …: RAR's own
    /// volumes, read through the tool from the first one.
    RarVolume,
}

/// What a part's name says about its set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartName {
    pub kind: SplitKind,
    /// The archive's own name: `backup.7z`, `site.zip`.
    pub base: String,
    /// 1-based part number (`.z01` is 1; the `.zip` of a spanned set has no number).
    pub number: u32,
    /// Digits in the part's suffix, for naming missing parts the same way.
    width: usize,
}

/// Parse a part name: `backup.7z.003` (numbered, its base an archive name), `site.z02`,
/// `data.part2.rar` or `data.r00`. Plain archive names (`site.zip`, `data.rar`) are not
/// parts by name.
pub fn parse(name: &str) -> Option<PartName> {
    let (stem, suffix) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    if suffix.eq_ignore_ascii_case("rar")
        && let Some((b, part)) = stem.rsplit_once('.')
        && let Some(digits) = part.get(4..).filter(|_| {
            part.get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("part"))
        })
        && !b.is_empty()
        && !digits.is_empty()
        && digits.bytes().all(|c| c.is_ascii_digit())
    {
        let number: u32 = digits.parse().ok()?;
        return (number >= 1).then(|| PartName {
            kind: SplitKind::RarVolume,
            base: format!("{b}.{suffix}"),
            number,
            width: digits.len(),
        });
    }
    if let Some(digits) = suffix.strip_prefix(['r', 'R'])
        && digits.len() >= 2
        && digits.bytes().all(|c| c.is_ascii_digit())
    {
        // `data.rar` is volume 1, `data.r00` volume 2.
        let n: u32 = digits.parse().ok()?;
        let rar = if suffix.starts_with('R') {
            "RAR"
        } else {
            "rar"
        };
        return Some(PartName {
            kind: SplitKind::RarVolume,
            base: format!("{stem}.{rar}"),
            number: n.checked_add(2)?,
            width: 0,
        });
    }
    if suffix.len() >= 3 && suffix.bytes().all(|b| b.is_ascii_digit()) {
        let number: u32 = suffix.parse().ok()?;
        let base_is_archive = by_name(stem).is_some() && stem.rsplit_once('.').is_some();
        if number >= 1 && base_is_archive {
            return Some(PartName {
                kind: SplitKind::Numbered,
                base: stem.to_string(),
                number,
                width: suffix.len(),
            });
        }
        return None;
    }
    let digits = suffix.strip_prefix(['z', 'Z'])?;
    if digits.len() >= 2 && digits.bytes().all(|b| b.is_ascii_digit()) {
        let number: u32 = digits.parse().ok()?;
        if number >= 1 {
            let zip = if suffix.starts_with('Z') {
                "ZIP"
            } else {
                "zip"
            };
            return Some(PartName {
                kind: SplitKind::ZipSpanned,
                base: format!("{stem}.{zip}"),
                number,
                width: digits.len(),
            });
        }
    }
    None
}

/// The file that stands for the whole set: the first numbered part, or the `.zip` of a
/// spanned set. Entering any part opens this one, so every part shares one mount.
pub fn canonical(name: &str) -> Option<String> {
    let p = parse(name)?;
    Some(match p.kind {
        SplitKind::Numbered => numbered_name(&p.base, 1, p.width),
        SplitKind::ZipSpanned => p.base,
        SplitKind::RarVolume => rar_first(name, &p),
    })
}

/// A part after the first (`.002` and up, any `.zNN`): listed with a part hint and not
/// shown as its own archive in the tree.
pub fn is_later_part(name: &str) -> bool {
    parse(name).is_some_and(|p| p.kind == SplitKind::ZipSpanned || p.number > 1)
}

/// "part 3" for the listing's type column (`None` for anything that is not a part).
pub fn part_label(name: &str) -> Option<String> {
    let p = parse(name)?;
    Some(format!("part {}", p.number))
}

/// The format of a numbered set (from its base name).
pub fn numbered_format(name: &str) -> Option<FormatId> {
    parse(name)
        .filter(|p| p.kind == SplitKind::Numbered)
        .and_then(|p| by_name(&p.base))
}

/// The first volume of the RAR set `p` (parsed from `name`).
fn rar_first(name: &str, p: &PartName) -> String {
    if p.width == 0 {
        return p.base.clone();
    }
    rar_part_name(name, p, 1)
}

/// `data.part07.rar` for volume `n`, keeping the spelling of `name`.
fn rar_part_name(name: &str, p: &PartName, n: u32) -> String {
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "rar"));
    let (b, part) = stem.rsplit_once('.').unwrap_or((stem, "part1"));
    let word = part.get(..4).unwrap_or("part");
    format!("{b}.{word}{n:0w$}.{ext}", w = p.width)
}

/// The volumes of the RAR set `first` (as [`canonical`] names it, or a plain `data.rar`)
/// belongs to, in order, from the names in its folder. A plain RAR with no volumes beside
/// it is a set of one. Fails naming the first missing volume.
pub fn resolve_rar<'a>(
    first: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, FmError> {
    let names: Vec<&str> = names.into_iter().collect();
    let new_style = parse(first).filter(|p| p.kind == SplitKind::RarVolume && p.width > 0);
    let base = new_style
        .as_ref()
        .map_or_else(|| first.to_string(), |p| p.base.clone());
    let mut found: Vec<(u32, String)> = names
        .iter()
        .filter_map(|n| {
            let q = parse(n).filter(|q| q.kind == SplitKind::RarVolume)?;
            let same = match &new_style {
                Some(p) => q.width > 0 && q.base == p.base,
                None => q.width == 0 && q.base.eq_ignore_ascii_case(first),
            };
            same.then(|| (q.number, (*n).to_string()))
        })
        .collect();
    if new_style.is_none() {
        found.push((1, first.to_string()));
    }
    found.sort();
    found.dedup_by_key(|(n, _)| *n);
    let last = found.last().map_or(1, |(n, _)| *n);
    if last > MAX_PARTS {
        return Err(too_many());
    }
    let mut parts = Vec::with_capacity(found.len());
    for want in 1..=last {
        match found.get(parts.len()) {
            Some((n, name)) if *n == want => parts.push(name.clone()),
            _ => {
                let name = match &new_style {
                    Some(p) => rar_part_name(first, p, want),
                    None => {
                        let stem = first.get(..first.len().saturating_sub(4)).unwrap_or(first);
                        format!("{stem}.r{:02}", want - 2)
                    }
                };
                return Err(missing(&base, want, &name));
            }
        }
    }
    Ok(parts)
}

fn numbered_name(base: &str, n: u32, width: usize) -> String {
    format!("{base}.{n:0width$}")
}

fn missing(base: &str, n: u32, part: &str) -> FmError {
    FmError::backend("archive", format!("Part {n} of {base} is missing ({part})"))
}

/// The parts of the numbered set `first` belongs to, in order, from the names in its
/// folder. Fails naming the first missing part.
pub fn resolve_numbered<'a>(
    first: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, FmError> {
    let p = parse(first)
        .filter(|p| p.kind == SplitKind::Numbered)
        .ok_or_else(|| FmError::InvalidParams(format!("{first} is not a numbered part")))?;
    let mut found: Vec<(u32, String)> = names
        .into_iter()
        .filter_map(|n| {
            let q = parse(n)?;
            (q.kind == SplitKind::Numbered && q.base == p.base).then(|| (q.number, n.to_string()))
        })
        .collect();
    found.sort();
    found.dedup_by_key(|(n, _)| *n);
    let last = found.last().map_or(0, |(n, _)| *n);
    if last > MAX_PARTS {
        return Err(too_many());
    }
    let mut parts = Vec::with_capacity(found.len());
    for want in 1..=last.max(1) {
        match found.get(parts.len()) {
            Some((n, name)) if *n == want => parts.push(name.clone()),
            _ => {
                let name = numbered_name(&p.base, want, p.width);
                return Err(missing(&p.base, want, &name));
            }
        }
    }
    Ok(parts)
}

/// The parts of the spanned zip `zip_name` (`site.zip`): `site.z01` … then `site.zip`.
pub fn resolve_spanned<'a>(
    zip_name: &str,
    disks: u32,
    names: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, FmError> {
    if disks > MAX_PARTS {
        return Err(too_many());
    }
    let stem = zip_name
        .get(..zip_name.len().saturating_sub(4))
        .filter(|_| zip_name.to_ascii_lowercase().ends_with(".zip"))
        .ok_or_else(|| FmError::InvalidParams(format!("{zip_name} is not a zip")))?;
    let names: Vec<&str> = names.into_iter().collect();
    let mut parts = Vec::with_capacity(disks as usize);
    // The `.zip` is disk number `disks`; the parts before it are `.z01` …
    for n in 1..disks {
        let hit = names.iter().find(|name| {
            parse(name).is_some_and(|p| {
                p.kind == SplitKind::ZipSpanned
                    && p.number == n
                    && p.base.eq_ignore_ascii_case(zip_name)
                    && name.get(..stem.len()) == Some(stem)
            })
        });
        match hit {
            Some(name) => parts.push((*name).to_string()),
            None => return Err(missing(zip_name, n, &format!("{stem}.z{n:02}"))),
        }
    }
    parts.push(zip_name.to_string());
    Ok(parts)
}

fn too_many() -> FmError {
    FmError::backend(
        "archive",
        format!("split archive has more than {MAX_PARTS} parts"),
    )
}

/// Numbered parts read as one file.
pub fn join(parts: Vec<Arc<dyn Source>>) -> Arc<dyn Source> {
    Arc::new(ConcatSource::new(parts))
}

const EOCD: [u8; 4] = *b"PK\x05\x06";
const EOCD64: [u8; 4] = *b"PK\x06\x06";
const LOC64: [u8; 4] = *b"PK\x06\x07";
const CEN: [u8; 4] = *b"PK\x01\x02";
/// Bytes at the end of a zip that can hold the end record (22 + 65 535 comment).
const TAIL: u64 = 22 + 65_535;
/// Largest central directory rewritten in memory.
const MAX_CD: u64 = 256 << 20;

fn read_exact(src: &dyn Source, at: u64, len: usize) -> Result<Vec<u8>, FmError> {
    let mut buf = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let n = src.read_at(at + done as u64, buf.get_mut(done..).unwrap_or(&mut []))?;
        if n == 0 {
            return Err(FmError::backend("archive", "zip end record is cut off"));
        }
        done += n;
    }
    Ok(buf)
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn put(b: &mut [u8], at: usize, bytes: &[u8]) -> Option<()> {
    b.get_mut(at..at + bytes.len())?.copy_from_slice(bytes);
    Some(())
}

/// The end-of-central-directory record of the zip in `last`: its offset and bytes.
fn find_eocd(last: &dyn Source) -> Result<Option<(u64, Vec<u8>)>, FmError> {
    let len = last.len();
    let start = len.saturating_sub(TAIL);
    let tail = read_exact(last, start, usize::try_from(len - start).unwrap_or(0))?;
    let Some(pos) = tail.windows(4).rposition(|w| w == EOCD) else {
        return Ok(None);
    };
    Ok(Some((
        start + pos as u64,
        tail.get(pos..).unwrap_or_default().to_vec(),
    )))
}

/// How many disks the zip whose last part is `last` spans (1 for an ordinary zip, `None`
/// when no end record is found).
pub fn zip_disks(last: &dyn Source) -> Result<Option<u32>, FmError> {
    Ok(find_eocd(last)?
        .and_then(|(_, e)| u16_at(&e, 4))
        .map(|d| u32::from(d) + 1))
}

/// Present the spanned zip `parts` (`.z01` … `.zip`) as one ordinary zip: the parts
/// joined, with the central directory's disk numbers set to 0 and its offsets moved
/// into the joined bytes (zip64 records included).
pub fn spanned_zip(parts: Vec<Arc<dyn Source>>) -> Result<Arc<dyn Source>, FmError> {
    let bad = || FmError::backend("archive", "the spanned zip's directory is damaged");
    let mut bases = Vec::with_capacity(parts.len());
    let mut at = 0u64;
    for p in &parts {
        bases.push(at);
        at = at.saturating_add(p.len());
    }
    let last = parts.last().ok_or_else(bad)?.clone();
    let (eocd_at, eocd) = find_eocd(last.as_ref())?.ok_or_else(bad)?;
    let base_of = |disk: u32| bases.get(disk as usize).copied().ok_or_else(bad);
    let joined = ConcatSource::new(parts.clone());

    // Classic end record fields, or the zip64 record they point to.
    let cd_disk = u32::from(u16_at(&eocd, 6).ok_or_else(bad)?);
    let mut cd_size = u64::from(u32_at(&eocd, 12).ok_or_else(bad)?);
    let mut cd_off = u64::from(u32_at(&eocd, 16).ok_or_else(bad)?);
    let mut cd_disk64 = cd_disk;
    let mut zip64: Option<Vec<u8>> = None;
    if eocd_at >= 20 {
        let loc = read_exact(last.as_ref(), eocd_at - 20, 20)?;
        if loc.get(..4) == Some(&LOC64) {
            let rec_disk = u32_at(&loc, 4).ok_or_else(bad)?;
            let rec_off = u64_at(&loc, 8).ok_or_else(bad)?;
            let rec_at = base_of(rec_disk)?.saturating_add(rec_off);
            let rec = read_exact(&joined, rec_at, 56)?;
            if rec.get(..4) != Some(&EOCD64) {
                return Err(bad());
            }
            cd_disk64 = u32_at(&rec, 20).ok_or_else(bad)?;
            cd_size = u64_at(&rec, 40).ok_or_else(bad)?;
            cd_off = u64_at(&rec, 48).ok_or_else(bad)?;
            zip64 = Some(rec);
        }
    }
    if cd_size > MAX_CD {
        return Err(FmError::backend(
            "archive",
            "the zip's directory is too large",
        ));
    }
    let cd_global = base_of(cd_disk64)?.saturating_add(cd_off);
    let mut cd = read_exact(
        &joined,
        cd_global,
        usize::try_from(cd_size).map_err(|_| bad())?,
    )?;

    // Rewrite every central header: disk 0, offset in the joined bytes.
    let mut i = 0usize;
    let mut entries = 0u64;
    while i + 46 <= cd.len() {
        if cd.get(i..i + 4) != Some(&CEN) {
            return Err(bad());
        }
        let name_len = usize::from(u16_at(&cd, i + 28).ok_or_else(bad)?);
        let extra_len = usize::from(u16_at(&cd, i + 30).ok_or_else(bad)?);
        let comment_len = usize::from(u16_at(&cd, i + 32).ok_or_else(bad)?);
        let mut disk = u32::from(u16_at(&cd, i + 34).ok_or_else(bad)?);
        let mut off = u64::from(u32_at(&cd, i + 42).ok_or_else(bad)?);
        let size32 = u32_at(&cd, i + 20).ok_or_else(bad)?;
        let csize32 = u32_at(&cd, i + 24).ok_or_else(bad)?;
        // A zip64 extra field holds, in order, the fields that are maxed out above.
        let extra_start = i + 46 + name_len;
        let mut off64_at = None;
        let mut disk64_at = None;
        let mut e = extra_start;
        while e + 4 <= extra_start + extra_len {
            let id = u16_at(&cd, e).ok_or_else(bad)?;
            let len = usize::from(u16_at(&cd, e + 2).ok_or_else(bad)?);
            if id == 0x0001 {
                let mut f = e + 4;
                if size32 == u32::MAX {
                    f += 8;
                }
                if csize32 == u32::MAX {
                    f += 8;
                }
                if off == u64::from(u32::MAX) {
                    off = u64_at(&cd, f).ok_or_else(bad)?;
                    off64_at = Some(f);
                    f += 8;
                }
                if disk == u32::from(u16::MAX) {
                    disk = u32_at(&cd, f).ok_or_else(bad)?;
                    disk64_at = Some(f);
                }
            }
            e += 4 + len;
        }
        let global = base_of(disk)?.saturating_add(off);
        match off64_at {
            Some(f) => put(&mut cd, f, &global.to_le_bytes()).ok_or_else(bad)?,
            None => {
                let small = u32::try_from(global).map_err(|_| {
                    FmError::backend("archive", "spanned zips over 4 GiB without zip64")
                })?;
                put(&mut cd, i + 42, &small.to_le_bytes()).ok_or_else(bad)?;
            }
        }
        if let Some(f) = disk64_at {
            put(&mut cd, f, &0u32.to_le_bytes()).ok_or_else(bad)?;
        }
        put(&mut cd, i + 34, &0u16.to_le_bytes()).ok_or_else(bad)?;
        i = extra_start + extra_len + comment_len;
        entries += 1;
    }

    // New end records: one disk, the directory at its joined offset.
    let mut tail = cd;
    if let Some(mut rec) = zip64 {
        let rec_at = cd_global.saturating_add(cd_size);
        put(&mut rec, 16, &0u32.to_le_bytes()).ok_or_else(bad)?;
        put(&mut rec, 20, &0u32.to_le_bytes()).ok_or_else(bad)?;
        put(&mut rec, 24, &entries.to_le_bytes()).ok_or_else(bad)?;
        put(&mut rec, 48, &cd_global.to_le_bytes()).ok_or_else(bad)?;
        tail.extend_from_slice(&rec);
        let mut loc = Vec::with_capacity(20);
        loc.extend_from_slice(&LOC64);
        loc.extend_from_slice(&0u32.to_le_bytes());
        loc.extend_from_slice(&rec_at.to_le_bytes());
        loc.extend_from_slice(&1u32.to_le_bytes());
        tail.extend_from_slice(&loc);
    }
    let mut end = eocd;
    put(&mut end, 4, &0u16.to_le_bytes()).ok_or_else(bad)?;
    put(&mut end, 6, &0u16.to_le_bytes()).ok_or_else(bad)?;
    let count = u16::try_from(entries).unwrap_or(u16::MAX);
    put(&mut end, 8, &count.to_le_bytes()).ok_or_else(bad)?;
    if let Ok(off32) = u32::try_from(cd_global) {
        if u32_at(&end, 16) != Some(u32::MAX) {
            put(&mut end, 16, &off32.to_le_bytes()).ok_or_else(bad)?;
        }
    } else if u32_at(&end, 16) != Some(u32::MAX) {
        return Err(FmError::backend(
            "archive",
            "spanned zips over 4 GiB without zip64",
        ));
    }
    tail.extend_from_slice(&end);
    // Everything before the directory, then the rewritten directory and end records.
    let head: Arc<dyn Source> = Arc::new(crate::source::SliceSource::new(
        Arc::new(joined),
        0,
        cd_global,
    ));
    Ok(Arc::new(ConcatSource::new(vec![
        head,
        Arc::new(MemSource::new(tail)),
    ])))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn part_names_are_recognised_only_for_archives() {
        let p = parse("backup.7z.003").expect("part");
        assert_eq!(
            (p.kind, p.base.as_str(), p.number),
            (SplitKind::Numbered, "backup.7z", 3)
        );
        assert_eq!(parse("site.tar.gz.0001").map(|p| p.number), Some(1));
        assert_eq!(
            parse("site.z02").map(|p| (p.kind, p.base, p.number)),
            Some((SplitKind::ZipSpanned, "site.zip".into(), 2))
        );
        for not_a_part in [
            "movie.mkv.001",
            "notes.001",
            "x.7z.01",
            "site.zip",
            ".001",
            "a.7z.000",
            "a.z1",
        ] {
            assert_eq!(parse(not_a_part), None, "{not_a_part}");
        }
        assert_eq!(canonical("backup.7z.003").as_deref(), Some("backup.7z.001"));
        assert_eq!(canonical("site.z02").as_deref(), Some("site.zip"));
        assert_eq!(canonical("SITE.Z01").as_deref(), Some("SITE.ZIP"));
        assert!(is_later_part("backup.7z.002") && !is_later_part("backup.7z.001"));
        assert!(is_later_part("site.z01") && !is_later_part("site.zip"));
        assert_eq!(part_label("backup.7z.012").as_deref(), Some("part 12"));
        assert_eq!(numbered_format("data.tar.gz.001"), Some(FormatId::TarGz));
    }

    #[test]
    fn rar_volumes_resolve_in_both_spellings() {
        assert_eq!(
            canonical("data.part2.rar").as_deref(),
            Some("data.part1.rar")
        );
        assert_eq!(
            canonical("data.PART03.RAR").as_deref(),
            Some("data.PART01.RAR")
        );
        assert_eq!(canonical("data.r00").as_deref(), Some("data.rar"));
        assert!(is_later_part("data.part2.rar") && !is_later_part("data.part1.rar"));
        assert!(is_later_part("data.r00"));
        assert_eq!(part_label("data.r00").as_deref(), Some("part 2"));
        assert_eq!(parse("data.rar"), None, "a plain rar is not a part by name");
        let new = [
            "data.part3.rar",
            "data.part1.rar",
            "data.part2.rar",
            "x.part1.rar",
        ];
        assert_eq!(
            resolve_rar("data.part1.rar", new).expect("set"),
            vec!["data.part1.rar", "data.part2.rar", "data.part3.rar"]
        );
        let err =
            resolve_rar("data.part1.rar", ["data.part1.rar", "data.part3.rar"]).expect_err("gap");
        assert!(
            err.to_string()
                .contains("Part 2 of data.rar is missing (data.part2.rar)"),
            "{err}"
        );
        let old = ["data.r01", "data.rar", "data.r00"];
        assert_eq!(
            resolve_rar("data.rar", old).expect("old"),
            vec!["data.rar", "data.r00", "data.r01"]
        );
        let err = resolve_rar("data.rar", ["data.rar", "data.r01"]).expect_err("gap");
        assert!(
            err.to_string()
                .contains("Part 2 of data.rar is missing (data.r00)"),
            "{err}"
        );
        assert_eq!(
            resolve_rar("single.rar", ["single.rar", "other.r00"]).expect("one"),
            vec!["single.rar"]
        );
    }

    #[test]
    fn numbered_sets_resolve_in_order_and_name_a_gap() {
        let names = [
            "backup.7z.002",
            "backup.7z.001",
            "other.7z.001",
            "backup.7z.003",
            "readme",
        ];
        assert_eq!(
            resolve_numbered("backup.7z.001", names).expect("set"),
            vec!["backup.7z.001", "backup.7z.002", "backup.7z.003"]
        );
        let gap = [
            "backup.7z.001",
            "backup.7z.002",
            "backup.7z.003",
            "backup.7z.005",
        ];
        let err = resolve_numbered("backup.7z.001", gap).expect_err("gap");
        assert!(
            err.to_string()
                .contains("Part 4 of backup.7z is missing (backup.7z.004)"),
            "{err}"
        );
        let first_gone = ["backup.7z.002"];
        assert!(resolve_numbered("backup.7z.001", first_gone).is_err());
    }

    #[test]
    fn spanned_sets_resolve_and_name_a_gap() {
        let names = ["site.z02", "site.zip", "site.z01", "site.txt"];
        assert_eq!(
            resolve_spanned("site.zip", 3, names).expect("set"),
            vec!["site.z01", "site.z02", "site.zip"]
        );
        let err = resolve_spanned("site.zip", 3, ["site.z01", "site.zip"]).expect_err("gap");
        assert!(
            err.to_string()
                .contains("Part 2 of site.zip is missing (site.z02)"),
            "{err}"
        );
        assert!(
            resolve_spanned("site.zip", 20_000, ["site.zip"]).is_err(),
            "too many"
        );
    }

    /// An ordinary zip cut into a spanned set the way Info-ZIP writes one: a spanning
    /// signature first, parts of `cut` bytes, the directory and end record on the last
    /// disk, every offset relative to its own disk.
    pub(crate) fn span(zip: &[u8], cut: usize) -> Vec<Vec<u8>> {
        let eocd = zip.windows(4).rposition(|w| w == EOCD).expect("eocd");
        let cd_off = u32_at(zip, eocd + 16).expect("cd") as usize;
        let mut data = b"PK\x07\x08".to_vec();
        data.extend_from_slice(&zip[..cd_off]);
        let mut parts: Vec<Vec<u8>> = data.chunks(cut).map(<[u8]>::to_vec).collect();
        let bases: Vec<usize> = parts
            .iter()
            .scan(0, |at, p| {
                let b = *at;
                *at += p.len();
                Some(b)
            })
            .collect();
        let disk_of = |g: usize| bases.iter().rposition(|b| *b <= g).expect("disk");
        let mut cd = zip[cd_off..eocd].to_vec();
        let mut i = 0;
        while i + 46 <= cd.len() {
            let g = u32_at(&cd, i + 42).expect("off") as usize + 4;
            let d = disk_of(g);
            put(&mut cd, i + 34, &(d as u16).to_le_bytes());
            put(&mut cd, i + 42, &((g - bases[d]) as u32).to_le_bytes());
            let n = usize::from(u16_at(&cd, i + 28).expect("n"));
            let x = usize::from(u16_at(&cd, i + 30).expect("x"));
            let c = usize::from(u16_at(&cd, i + 32).expect("c"));
            i += 46 + n + x + c;
        }
        let last = parts.len() - 1;
        let mut end = zip[eocd..].to_vec();
        put(&mut end, 4, &(last as u16).to_le_bytes());
        put(&mut end, 6, &(last as u16).to_le_bytes());
        let cd_in_last = parts[last].len() as u32;
        put(&mut end, 16, &cd_in_last.to_le_bytes());
        parts[last].extend_from_slice(&cd);
        parts[last].extend_from_slice(&end);
        parts
    }

    #[test]
    fn a_spanned_zip_reads_as_one_archive() {
        use std::io::{Read as _, Write as _};
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let mut x = 0x2545_f491_u32;
        let noise: Vec<u8> = (0..9000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        for (name, data) in [
            ("a.txt", &b"alpha"[..]),
            ("dir/noise.bin", &noise[..]),
            ("z.txt", &b"zulu"[..]),
        ] {
            w.start_file(name, opts).expect("start");
            w.write_all(data).expect("w");
        }
        let zip = w.finish().expect("finish").into_inner();
        let parts = span(&zip, 2000);
        assert!(parts.len() >= 4, "{} parts", parts.len());
        let last = MemSource::new(parts.last().expect("last").clone());
        assert_eq!(zip_disks(&last).expect("eocd"), Some(parts.len() as u32));
        let sources: Vec<Arc<dyn Source>> = parts
            .into_iter()
            .map(|p| Arc::new(MemSource::new(p)) as Arc<dyn Source>)
            .collect();
        let joined = spanned_zip(sources).expect("joined");
        let mut archive =
            zip::ZipArchive::new(crate::source::SourceReader::new(joined)).expect("reads as a zip");
        for (name, want) in [
            ("a.txt", &b"alpha"[..]),
            ("dir/noise.bin", &noise[..]),
            ("z.txt", &b"zulu"[..]),
        ] {
            let mut got = Vec::new();
            archive
                .by_name(name)
                .expect(name)
                .read_to_end(&mut got)
                .expect("read");
            assert_eq!(got, want, "{name}");
        }
        let ordinary = MemSource::new(zip);
        assert_eq!(zip_disks(&ordinary).expect("eocd"), Some(1));
    }

    #[test]
    fn a_damaged_spanned_directory_is_an_error_not_a_panic() {
        let mut parts = vec![b"PK\x07\x08junk".to_vec(), b"more".to_vec()];
        let mut end = EOCD.to_vec();
        end.extend_from_slice(&[1, 0, 1, 0, 1, 0, 1, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 0]);
        parts[1].extend_from_slice(&end);
        let sources: Vec<Arc<dyn Source>> = parts
            .into_iter()
            .map(|p| Arc::new(MemSource::new(p)) as Arc<dyn Source>)
            .collect();
        assert!(spanned_zip(sources).is_err());
    }

    #[test]
    fn hostile_part_names_and_counts_stay_bounded() {
        let many: Vec<String> = (1..=MAX_PARTS + 5)
            .map(|n| format!("b.7z.{n:05}"))
            .collect();
        let err = resolve_numbered("b.7z.00001", many.iter().map(String::as_str)).expect_err("cap");
        assert!(err.to_string().contains("more than"), "{err}");
        assert_eq!(
            parse("b.7z.99999999999999999999"),
            None,
            "overflowing number"
        );
        assert_eq!(
            parse("../x.7z.001").map(|p| p.base),
            Some("../x.7z".into()),
            "name only, never a path"
        );
    }
}
