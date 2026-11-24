use crate::error::ShardError;
use anyhow::Result;
use std::io::Read;
use std::str::FromStr;

/// Default maximum decompressed chunk size (64 MiB) to defend against decompression bombs.
pub const DEFAULT_MAX_DECOMPRESSED_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Zstd,
    Zlib,
}

impl FromStr for Compression {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "none" => Ok(Compression::None),
            "zstd" => Ok(Compression::Zstd),
            "zlib" => Ok(Compression::Zlib),
            other => anyhow::bail!("Unknown compression algorithm: {other}"),
        }
    }
}

impl Compression {
    pub fn as_str(&self) -> &'static str {
        match self {
            Compression::None => "none",
            Compression::Zstd => "zstd",
            Compression::Zlib => "zlib",
        }
    }

    pub fn code(&self) -> u8 {
        match self {
            Compression::None => 0,
            Compression::Zstd => 1,
            Compression::Zlib => 2,
        }
    }

    pub fn from_code(code: u8) -> Result<Self> {
        match code {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Zstd),
            2 => Ok(Compression::Zlib),
            other => anyhow::bail!("Unknown compression code: {other}"),
        }
    }

    pub fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Compression::None => Ok(data.to_vec()),
            Compression::Zstd => {
                let compressed = zstd::bulk::compress(data, 3)?;
                Ok(compressed)
            }
            Compression::Zlib => {
                use std::io::Write;
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(data)?;
                let compressed = encoder.finish()?;
                Ok(compressed)
            }
        }
    }

    /// Decompress `data` using bounded streaming decompression up to
    /// [`DEFAULT_MAX_DECOMPRESSED_SIZE`] (64 MiB).
    pub fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        self.decompress_bounded(data, DEFAULT_MAX_DECOMPRESSED_SIZE)
    }

    /// Decompress `data` with an explicit upper bound `max_bytes` on output size.
    /// Streams incrementally rather than pre-allocating `max_bytes`.
    pub fn decompress_bounded(&self, data: &[u8], max_bytes: usize) -> Result<Vec<u8>> {
        match self {
            Compression::None => {
                if data.len() > max_bytes {
                    return Err(ShardError::DecompressionLimitExceeded(max_bytes).into());
                }
                Ok(data.to_vec())
            }
            Compression::Zstd => {
                let decoder = zstd::stream::read::Decoder::new(data)?;
                read_bounded(decoder, max_bytes)
            }
            Compression::Zlib => {
                let decoder = flate2::read::ZlibDecoder::new(data);
                read_bounded(decoder, max_bytes)
            }
        }
    }
}

fn read_bounded<R: Read>(reader: R, max_bytes: usize) -> Result<Vec<u8>> {
    let limit = (max_bytes as u64).saturating_add(1);
    let mut limited = reader.take(limit);
    let mut buf = Vec::new();
    limited.read_to_end(&mut buf)?;
    if buf.len() > max_bytes {
        return Err(ShardError::DecompressionLimitExceeded(max_bytes).into());
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress_none() {
        let data = b"hello world";
        let compressed = Compression::None.compress(data).unwrap();
        assert_eq!(compressed, data);
        let decompressed = Compression::None.decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compress_decompress_zstd() {
        let data = vec![0u8; 4096];
        let compressed = Compression::Zstd.compress(&data).unwrap();
        assert!(compressed.len() < data.len(), "zstd should compress zeros");
        let decompressed = Compression::Zstd.decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compress_decompress_zlib() {
        let data = vec![0u8; 4096];
        let compressed = Compression::Zlib.compress(&data).unwrap();
        assert!(compressed.len() < data.len(), "zlib should compress zeros");
        let decompressed = Compression::Zlib.decompress(&compressed).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_decompression_bomb_rejected() {
        let payload = vec![0u8; 1024 * 1024]; // 1 MiB of zeros
        let zstd_bomb = Compression::Zstd.compress(&payload).unwrap();
        let zlib_bomb = Compression::Zlib.compress(&payload).unwrap();

        assert!(Compression::Zstd
            .decompress_bounded(&zstd_bomb, 4096)
            .is_err());
        assert!(Compression::Zlib
            .decompress_bounded(&zlib_bomb, 4096)
            .is_err());
        assert!(Compression::None
            .decompress_bounded(&payload, 4096)
            .is_err());

        // Exact limit succeeds
        assert_eq!(
            Compression::Zstd
                .decompress_bounded(&zstd_bomb, payload.len())
                .unwrap()
                .len(),
            payload.len()
        );
    }

    #[test]
    fn test_roundtrip_all() {
        let data = b"The quick brown fox jumps over the lazy dog";
        for algo in &[Compression::None, Compression::Zstd, Compression::Zlib] {
            let compressed = algo.compress(data).unwrap();
            let decompressed = algo.decompress(&compressed).unwrap();
            assert_eq!(decompressed, data, "roundtrip failed for {:?}", algo);
        }
    }

    #[test]
    fn test_from_str() {
        assert_eq!("none".parse::<Compression>().unwrap(), Compression::None);
        assert_eq!("zstd".parse::<Compression>().unwrap(), Compression::Zstd);
        assert_eq!("zlib".parse::<Compression>().unwrap(), Compression::Zlib);
        assert!("invalid".parse::<Compression>().is_err());
    }

    #[test]
    fn test_as_str() {
        assert_eq!(Compression::None.as_str(), "none");
        assert_eq!(Compression::Zstd.as_str(), "zstd");
        assert_eq!(Compression::Zlib.as_str(), "zlib");
    }
}
