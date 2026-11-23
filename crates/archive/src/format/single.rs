//! A single compressed file (`access.log.gz`) shown as a folder holding that one file.
//! A `.gz`/`.xz`/… whose content is a tar opens as that tar.

use std::io::Read;
use std::sync::Arc;

use fm_vfs::FmError;

use super::compress::decoder;
use super::{FormatId, Info, Passwords, Progress, Reader};
use crate::index::{Index, Meta, NodeId, NodeKind};
use crate::source::{Source, SourceReader};

pub fn open(
    src: Arc<dyn Source>,
    format: FormatId,
    name: &str,
    progress: &Progress<'_>,
) -> Result<Box<dyn Reader>, FmError> {
    let codec = format
        .compression()
        .ok_or(FmError::Unsupported("not a compressed file"))?;
    // A tar inside? Look at the first block of the decompressed data.
    let mut head = Vec::with_capacity(512);
    decoder(codec, SourceReader::new(src.clone()))?
        .take(512)
        .read_to_end(&mut head)
        .map_err(|e| FmError::backend("archive", format!("{name}: {e}")))?;
    if head.get(257..262) == Some(b"ustar") {
        let tar_format = match format {
            FormatId::Gz => FormatId::TarGz,
            FormatId::Bz2 => FormatId::TarBz2,
            FormatId::Xz => FormatId::TarXz,
            _ => FormatId::TarZst,
        };
        return Ok(Box::new(super::tar::TarReader::open(
            src, tar_format, progress,
        )?));
    }
    let inner = inner_name(name, format);
    let mut index = Index::new();
    index.add(
        &inner,
        NodeKind::File,
        Meta {
            size: gzip_size(src.as_ref(), format),
            packed: Some(src.len()),
            ..Meta::default()
        },
    )?;
    Ok(Box::new(Single { src, format, index }))
}

/// `access.log.gz` → `access.log`; a name without the suffix gets `.out`.
pub fn inner_name(name: &str, format: FormatId) -> String {
    let suffix = match format {
        FormatId::Gz => ".gz",
        FormatId::Bz2 => ".bz2",
        FormatId::Xz => ".xz",
        _ => ".zst",
    };
    let lower = name.to_ascii_lowercase();
    if lower.len() > suffix.len() && lower.ends_with(suffix) {
        name.get(..name.len() - suffix.len())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{name}.out"))
    } else {
        format!("{name}.out")
    }
}

/// gzip stores the size modulo 4 GiB in its last four bytes; trust it only below that.
fn gzip_size(src: &dyn Source, format: FormatId) -> Option<u64> {
    if format != FormatId::Gz || src.len() < 18 {
        return None;
    }
    let mut tail = [0u8; 4];
    let n = src.read_at(src.len() - 4, &mut tail).ok()?;
    (n == 4).then(|| u64::from(u32::from_le_bytes(tail)))
}

struct Single {
    src: Arc<dyn Source>,
    format: FormatId,
    index: Index,
}

impl Reader for Single {
    fn format(&self) -> FormatId {
        self.format
    }

    fn index(&self) -> &Index {
        &self.index
    }

    fn info(&self) -> Info {
        Info {
            methods: self
                .format
                .compression()
                .map(|c| vec![c.label().to_string()])
                .unwrap_or_default(),
            ..Info::default()
        }
    }

    fn open(&self, id: NodeId, _: &Passwords) -> Result<Box<dyn Read + Send>, FmError> {
        if self.index.node(id).is_none_or(|n| n.kind != NodeKind::File) {
            return Err(FmError::NotFound("archive member".into()));
        }
        let codec = self
            .format
            .compression()
            .ok_or(FmError::Unsupported("not a compressed file"))?;
        decoder(codec, SourceReader::new(self.src.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemSource;
    use std::io::Write as _;

    #[test]
    fn a_gzipped_log_is_a_folder_with_one_file() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(b"line 1\nline 2\n").expect("gz");
        let src = Arc::new(MemSource::new(gz.finish().expect("gz")));
        let r = open(src, FormatId::Gz, "access.log.gz", &Progress::none()).expect("open");
        let id = r.index().lookup("access.log").expect("inner");
        assert_eq!(r.index().node(id).and_then(|n| n.meta.size), Some(14));
        let mut s = String::new();
        r.open(id, &Passwords::default())
            .expect("open")
            .read_to_string(&mut s)
            .expect("read");
        assert_eq!(s, "line 1\nline 2\n");
    }

    #[test]
    fn a_gz_holding_a_tar_opens_as_the_tar() {
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&super::super::tar::tests::sample_tar())
            .expect("gz");
        let src = Arc::new(MemSource::new(gz.finish().expect("gz")));
        let r = open(src, FormatId::Gz, "backup.gz", &Progress::none()).expect("open");
        assert_eq!(r.format(), FormatId::TarGz);
        assert!(r.index().lookup("www/index.html").is_some());
    }

    #[test]
    fn inner_names() {
        assert_eq!(inner_name("a.LOG.GZ", FormatId::Gz), "a.LOG");
        assert_eq!(inner_name("blob", FormatId::Xz), "blob.out");
    }
}
