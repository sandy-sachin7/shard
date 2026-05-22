use anyhow::Result;
use std::str::FromStr;

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

    pub fn decompress(&self, data: &[u8]) -> Result<Vec<u8>> {
        match self {
            Compression::None => Ok(data.to_vec()),
            Compression::Zstd => {
                let decompressed = zstd::bulk::decompress(data, 1024 * 1024 * 1024)?;
                Ok(decompressed)
            }
            Compression::Zlib => {
                use std::io::Read;
                let mut decoder = flate2::read::ZlibDecoder::new(data);
                let mut buf = Vec::new();
                decoder.read_to_end(&mut buf)?;
                Ok(buf)
            }
        }
    }
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
