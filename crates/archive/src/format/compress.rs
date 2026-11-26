//! Stream codecs around tars and single files: gzip, bzip2, xz, Zstandard (pure Rust).

use std::io::{self, Read};

use fm_vfs::FmError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Gzip,
    Bzip2,
    Xz,
    Zstd,
}

impl Codec {
    pub fn label(self) -> &'static str {
        match self {
            Self::Gzip => "gzip",
            Self::Bzip2 => "bzip2",
            Self::Xz => "xz",
            Self::Zstd => "Zstandard",
        }
    }
}

/// Largest Zstandard window accepted (the format's default limit for decoders is 8 MiB;
/// `--long` archives use up to 128 MiB).
const ZSTD_MAX_WINDOW: u64 = 128 << 20;

/// A decoder over `inner` that reads every concatenated stream/frame to the end.
pub fn decoder<R: Read + Send + 'static>(
    codec: Codec,
    inner: R,
) -> Result<Box<dyn Read + Send>, FmError> {
    Ok(match codec {
        Codec::Gzip => Box::new(flate2::read::MultiGzDecoder::new(inner)),
        Codec::Bzip2 => Box::new(bzip2::read::MultiBzDecoder::new(inner)),
        Codec::Xz => Box::new(lzma_rust2::XzReader::new(inner, true)),
        Codec::Zstd => Box::new(ZstdFrames::new(inner)),
    })
}

/// Decodes one Zstandard frame after another until the input ends.
struct ZstdFrames<R: Read> {
    inner: Option<Peek<R>>,
    frame: Option<ruzstd::decoding::StreamingDecoder<Peek<R>, ruzstd::decoding::FrameDecoder>>,
}

impl<R: Read> ZstdFrames<R> {
    fn new(inner: R) -> Self {
        Self {
            inner: Some(Peek { first: None, inner }),
            frame: None,
        }
    }
}

impl<R: Read> Read for ZstdFrames<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if let Some(frame) = self.frame.as_mut() {
                let n = frame.read(out)?;
                if n > 0 {
                    return Ok(n);
                }
                // This frame is done: take the input back and look for another frame.
                self.inner = self.frame.take().map(|f| f.into_inner());
            }
            let Some(mut inner) = self.inner.take() else {
                return Ok(0);
            };
            let mut first = [0u8; 1];
            if inner.read(&mut first)? == 0 {
                return Ok(0);
            }
            inner.first = first.first().copied();
            let frame = ruzstd::decoding::StreamingDecoder::new_with_max_window_size(
                inner,
                ZSTD_MAX_WINDOW,
            )
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
            self.frame = Some(frame);
        }
    }
}

/// A reader that can give back one byte it already read.
struct Peek<R: Read> {
    first: Option<u8>,
    inner: R,
}

impl<R: Read> Read for Peek<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(b) = self.first.take() {
            if let Some(slot) = out.first_mut() {
                *slot = b;
                return Ok(1);
            }
            self.first = Some(b);
            return Ok(0);
        }
        self.inner.read(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spike 1.4 (manual, `--release`): compress 100 MB of tar-like data with `ruzstd`,
    /// time it, decode it again with `ruzstd`, and leave the output at `$FM_ZSTD_PROBE`
    /// for the reference decoder (`zstd -t`). Results are in design.md (D11).
    #[test]
    #[ignore = "manual speed probe; run with --release and FM_ZSTD_PROBE=<file>"]
    fn ruzstd_encoder_probe() {
        use std::time::Instant;
        let mut data = Vec::with_capacity(100 << 20);
        let mut seed = 0x9e37_79b9_u32;
        let mut line = 0u32;
        while data.len() < 100 << 20 {
            // Log-like lines with some noise: compressible, not trivially so.
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            line += 1;
            data.extend_from_slice(
                format!(
                    "{line:08} GET /api/items/{} 200 {}ms\n",
                    seed % 100_000,
                    seed % 997
                )
                .as_bytes(),
            );
        }
        let started = Instant::now();
        let packed = ruzstd::encoding::compress_to_vec(
            &data[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        let secs = started.elapsed().as_secs_f64();
        let started = Instant::now();
        let back = decode(Codec::Zstd, packed.clone());
        let back_secs = started.elapsed().as_secs_f64();
        assert_eq!(back, data, "round trip");
        println!(
            "ruzstd Fastest: {} MB -> {} MB ({:.1} %), encode {:.0} MB/s, decode {:.0} MB/s",
            data.len() >> 20,
            packed.len() >> 20,
            packed.len() as f64 * 100.0 / data.len() as f64,
            (data.len() >> 20) as f64 / secs,
            (data.len() >> 20) as f64 / back_secs,
        );
        if let Some(path) = std::env::var_os("FM_ZSTD_PROBE") {
            std::fs::write(path, &packed).expect("write probe output");
        }
    }

    fn decode(codec: Codec, data: Vec<u8>) -> Vec<u8> {
        let mut out = Vec::new();
        decoder(codec, io::Cursor::new(data))
            .expect("decoder")
            .read_to_end(&mut out)
            .expect("read");
        out
    }

    #[test]
    fn concatenated_streams_decode_fully() {
        use std::io::Write as _;
        let gz = |text: &[u8]| {
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            e.write_all(text).expect("w");
            e.finish().expect("finish")
        };
        let mut two = gz(b"hello ");
        two.extend(gz(b"world"));
        assert_eq!(decode(Codec::Gzip, two), b"hello world");

        let zst = |text: &[u8]| {
            ruzstd::encoding::compress_to_vec(text, ruzstd::encoding::CompressionLevel::Fastest)
        };
        let mut two = zst(b"split ");
        two.extend(zst(b"frames"));
        assert_eq!(decode(Codec::Zstd, two), b"split frames");

        let mut xz = Vec::new();
        {
            let mut w =
                lzma_rust2::XzWriter::new(&mut xz, lzma_rust2::XzOptions::default()).expect("xz");
            w.write_all(b"xz data").expect("w");
            w.finish().expect("finish");
        }
        assert_eq!(decode(Codec::Xz, xz), b"xz data");

        let mut bz = Vec::new();
        {
            let mut w = bzip2::write::BzEncoder::new(&mut bz, bzip2::Compression::fast());
            w.write_all(b"bz data").expect("w");
            w.finish().expect("finish");
        }
        assert_eq!(decode(Codec::Bzip2, bz), b"bz data");
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        for codec in [Codec::Gzip, Codec::Bzip2, Codec::Xz, Codec::Zstd] {
            let mut out = Vec::new();
            let r = decoder(codec, io::Cursor::new(b"not compressed at all".to_vec()))
                .and_then(|mut d| d.read_to_end(&mut out).map_err(FmError::from));
            assert!(r.is_err(), "{codec:?}");
        }
    }
}
