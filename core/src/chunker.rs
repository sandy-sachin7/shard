use anyhow::Result;
use std::collections::BTreeMap;
use std::io::Read;

/// Maximum permitted chunk size (16 MiB).
pub const MAX_CHUNK_SIZE: usize = 16 * 1024 * 1024;

/// A chunk produced by the chunker: Blake3 content hash, raw bytes, and byte offset in the original file.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Blake3 hash of the original (uncompressed) data.
    pub hash: blake3::Hash,
    /// Raw (uncompressed) byte content of the chunk.
    pub data: Vec<u8>,
    /// Byte offset of this chunk in the original file, used for ordering.
    pub offset: u64,
}

// ── Fixed-size chunker ─────────────────────────────────────────────────────

/// Chunker that emits chunks of a fixed byte size.
pub struct FixedChunker {
    reader: Box<dyn Read + Send>,
    offset: u64,
    chunk_size: usize,
}

impl FixedChunker {
    fn new(reader: Box<dyn Read + Send>, chunk_size: usize) -> Self {
        let safe_size = chunk_size.clamp(1, MAX_CHUNK_SIZE);
        Self {
            reader,
            offset: 0,
            chunk_size: safe_size,
        }
    }

    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        let mut buffer = vec![0u8; self.chunk_size];
        let mut bytes_read = 0;

        while bytes_read < self.chunk_size {
            let n = self.reader.read(&mut buffer[bytes_read..])?;
            if n == 0 {
                break;
            }
            bytes_read += n;
        }

        if bytes_read == 0 {
            return Ok(None);
        }

        buffer.truncate(bytes_read);
        let hash = blake3::hash(&buffer);
        let chunk = Chunk {
            hash,
            data: buffer,
            offset: self.offset,
        };

        self.offset += bytes_read as u64;
        Ok(Some(chunk))
    }
}

// ── FastCDC Content-Defined Chunker (64-bit Gear hash + dual normalized masks) ─

/// 256 deterministic 64-bit pseudo-random constants (SplitMix64 seeded table)
/// used for shift-invariant FastCDC Gear hashing.
const GEAR_TABLE: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut i = 0;
    while i < 256 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
};

/// Construct a 64-bit Gear mask with `bits` distributed high-order bits set,
/// so `(fp & mask) == 0` triggers with probability `2^(-bits)` independently of
/// absolute file offset (true shift-invariant CDC).
fn gear_mask(bits: u32) -> u64 {
    let bits = bits.clamp(1, 32);
    // Use the upper bits of the 64-bit Gear fingerprint since Gear shifts left
    // (`fp = (fp << 1).wrapping_add(GEAR[b])`), which mixes the last 64 bytes
    // into the high bits.
    !0u64 << (64 - bits)
}

/// Content-defined chunker implementing FastCDC (Gear rolling hash with normalized
/// dual masks `mask_s` for `[min, avg)` and `mask_l` for `[avg, max]`) backed by a
/// cursor buffer to avoid per-chunk `Vec::drain` memmoves.
pub struct FastCdcChunker {
    reader: Box<dyn Read + Send>,
    offset: u64,
    min_size: usize,
    avg_size: usize,
    max_size: usize,
    mask_s: u64,
    mask_l: u64,
    buf: Vec<u8>,
    pos: usize,
    filled: usize,
    eof: bool,
}

/// Legacy alias for [`FastCdcChunker`].
pub type RabinChunker = FastCdcChunker;

impl FastCdcChunker {
    fn new(reader: Box<dyn Read + Send>, min: usize, avg: usize, max: usize) -> Self {
        let min_size = min.clamp(1, MAX_CHUNK_SIZE);
        let avg_size = avg.clamp(min_size, MAX_CHUNK_SIZE);
        let max_size = max.clamp(avg_size, MAX_CHUNK_SIZE);

        let avg_bits = (avg_size.next_power_of_two().trailing_zeros()).max(1);
        // Normalized chunking (FastCDC NC=1): stricter mask before avg, looser mask after avg.
        let mask_s = gear_mask(avg_bits.saturating_add(1));
        let mask_l = gear_mask(avg_bits.saturating_sub(1).max(1));

        let cap = max_size.saturating_mul(2).max(65_536);
        Self {
            reader,
            offset: 0,
            min_size,
            avg_size,
            max_size,
            mask_s,
            mask_l,
            buf: vec![0u8; cap],
            pos: 0,
            filled: 0,
            eof: false,
        }
    }

    fn available(&self) -> usize {
        self.filled - self.pos
    }

    fn fill_buf(&mut self) -> Result<()> {
        if self.eof || self.available() >= self.max_size {
            return Ok(());
        }
        // Compact remaining unread bytes to the start only when needed.
        if self.pos > 0 {
            let rem = self.available();
            if rem > 0 {
                self.buf.copy_within(self.pos..self.filled, 0);
            }
            self.pos = 0;
            self.filled = rem;
        }
        while self.filled < self.buf.len() {
            let n = self.reader.read(&mut self.buf[self.filled..])?;
            if n == 0 {
                self.eof = true;
                break;
            }
            self.filled += n;
            if self.available() >= self.max_size {
                break;
            }
        }
        Ok(())
    }

    fn find_cut_point(&self, slice: &[u8]) -> usize {
        let len = slice.len();
        if len <= self.min_size {
            return len;
        }
        let max_cut = len.min(self.max_size);
        let normal_end = self.avg_size.min(max_cut);

        let mut fp: u64 = 0;
        // Start scanning from `min_size` (Gear hash depends solely on the last <=64 bytes
        // preceding each candidate cut position, so boundaries are shift-invariant).
        let mut i = self.min_size;
        while i < normal_end {
            fp = (fp << 1).wrapping_add(GEAR_TABLE[slice[i] as usize]);
            if (fp & self.mask_s) == 0 {
                return i + 1;
            }
            i += 1;
        }
        while i < max_cut {
            fp = (fp << 1).wrapping_add(GEAR_TABLE[slice[i] as usize]);
            if (fp & self.mask_l) == 0 {
                return i + 1;
            }
            i += 1;
        }
        max_cut
    }

    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        self.fill_buf()?;

        let avail = self.available();
        if avail == 0 {
            return Ok(None);
        }

        let slice = &self.buf[self.pos..self.filled];
        let cut = if self.eof && avail <= self.min_size {
            avail
        } else {
            self.find_cut_point(slice)
        };

        let data = slice[..cut].to_vec();
        let hash = blake3::hash(&data);
        let chunk = Chunk {
            hash,
            data,
            offset: self.offset,
        };

        self.pos += cut;
        self.offset += cut as u64;
        Ok(Some(chunk))
    }
}

// ── Unified chunker API ────────────────────────────────────────────────────

/// Chunker mode selected at init time — either fixed-size or content-defined (FastCDC).
/// Deserialized from `.shard/config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkerMode {
    /// Emit chunks of exactly `chunk_size` bytes.
    Fixed { chunk_size: u64 },
    /// Emit variable-size chunks via FastCDC Gear rolling hash, with min/avg/max bounds.
    FastCdc { min: u64, avg: u64, max: u64 },
}

impl ChunkerMode {
    /// Parse a `ChunkerMode` from the repository config JSON.
    /// Accepts both `"fastcdc"` and legacy `"rabin"` for content-defined chunking.
    /// Defaults to `Fixed { chunk_size: 4_194_304 }` if `chunker_mode` is absent.
    pub fn from_config(config: &BTreeMap<String, String>) -> Self {
        match config.get("chunker_mode").map(|s| s.as_str()) {
            Some("fastcdc") | Some("rabin") => ChunkerMode::FastCdc {
                min: config
                    .get("chunk_min")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1_048_576),
                avg: config
                    .get("chunk_avg")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(4_194_304),
                max: config
                    .get("chunk_max")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(8_388_608),
            },
            _ => ChunkerMode::Fixed {
                chunk_size: config
                    .get("chunk_size")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(4_194_304),
            },
        }
    }
}

/// Unified chunker interface — dispatches to either [`FixedChunker`] or [`FastCdcChunker`].
pub enum Chunker {
    Fixed(FixedChunker),
    FastCdc(FastCdcChunker),
}

impl Chunker {
    /// Create a new fixed-size chunker reading from `reader`.
    pub fn new_fixed(reader: Box<dyn Read + Send>, chunk_size: u64) -> Self {
        Chunker::Fixed(FixedChunker::new(reader, chunk_size as usize))
    }

    /// Create a new FastCDC content-defined chunker reading from `reader`.
    pub fn new_fastcdc(reader: Box<dyn Read + Send>, min: u64, avg: u64, max: u64) -> Self {
        Chunker::FastCdc(FastCdcChunker::new(
            reader,
            min as usize,
            avg as usize,
            max as usize,
        ))
    }

    /// Legacy alias for [`Chunker::new_fastcdc`].
    pub fn new_rabin(reader: Box<dyn Read + Send>, min: u64, avg: u64, max: u64) -> Self {
        Self::new_fastcdc(reader, min, avg, max)
    }

    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        match self {
            Chunker::Fixed(c) => c.next_chunk(),
            Chunker::FastCdc(c) => c.next_chunk(),
        }
    }
}

// ── Validation ─────────────────────────────────────────────────────────────

/// Validate that a chunk size is within the acceptable range (1 to 16 MiB inclusive).
pub fn validate_chunk_size(size: usize) -> Result<(), String> {
    if size == 0 {
        Err("chunk size must be greater than 0".to_string())
    } else if size > MAX_CHUNK_SIZE {
        Err(format!(
            "chunk size {} exceeds maximum of {}",
            size, MAX_CHUNK_SIZE
        ))
    } else {
        Ok(())
    }
}

/// Validate FastCDC `(min, avg, max)` bounds: `0 < min <= avg <= max <= MAX_CHUNK_SIZE`.
pub fn validate_fastcdc_params(min: usize, avg: usize, max: usize) -> Result<(), String> {
    if min == 0 {
        return Err("chunk_min must be greater than 0".to_string());
    }
    if min > avg {
        return Err(format!(
            "chunk_min ({}) must be <= chunk_avg ({})",
            min, avg
        ));
    }
    if avg > max {
        return Err(format!(
            "chunk_avg ({}) must be <= chunk_max ({})",
            avg, max
        ));
    }
    if max > MAX_CHUNK_SIZE {
        return Err(format!(
            "chunk_max ({}) exceeds maximum of {}",
            max, MAX_CHUNK_SIZE
        ));
    }
    Ok(())
}

/// Legacy alias for [`validate_fastcdc_params`].
pub fn validate_rabin_params(min: usize, avg: usize, max: usize) -> Result<(), String> {
    validate_fastcdc_params(min, avg, max)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Cursor;

    fn pseudo_random_bytes(len: usize, seed: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut s = seed;
        for _ in 0..len {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            out.push((s >> 33) as u8);
        }
        out
    }

    #[test]
    fn test_fixed_chunking_small() {
        let data = vec![0u8; 1024];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let chunk = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.data.len(), 1024);
        assert_eq!(chunk.offset, 0);
        assert!(chunker.next_chunk().unwrap().is_none());
    }

    #[test]
    fn test_fixed_chunking_exact() {
        let data = vec![0u8; 4_194_304];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let chunk = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.data.len(), 4_194_304);
        assert_eq!(chunk.offset, 0);
        assert!(chunker.next_chunk().unwrap().is_none());
    }

    #[test]
    fn test_fixed_chunking_large() {
        let data = vec![0u8; 4_194_304 + 1024];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let chunk1 = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk1.data.len(), 4_194_304);
        assert_eq!(chunk1.offset, 0);
        let chunk2 = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk2.data.len(), 1024);
        assert_eq!(chunk2.offset, 4_194_304);
        assert!(chunker.next_chunk().unwrap().is_none());
    }

    #[test]
    fn test_fixed_chunking_empty() {
        let data: Vec<u8> = vec![];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        assert!(chunker.next_chunk().unwrap().is_none());
    }

    #[test]
    fn test_rabin_chunking_small() {
        let data = vec![0u8; 1024];
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data)), 256, 512, 1024);
        let mut total = 0;
        while let Some(chunk) = chunker.next_chunk().unwrap() {
            total += chunk.data.len();
        }
        assert_eq!(total, 1024);
    }

    #[test]
    fn test_rabin_small_min_does_not_panic() {
        // Previously panicked when min_size < 31!
        let data = pseudo_random_bytes(1024, 99);
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data.clone())), 8, 32, 128);
        let mut reassembled = Vec::new();
        while let Some(c) = chunker.next_chunk().unwrap() {
            assert!(c.data.len() <= 128);
            reassembled.extend_from_slice(&c.data);
        }
        assert_eq!(reassembled, data);
    }

    #[test]
    fn test_rabin_chunking_produces_multiple_chunks() {
        let data = pseudo_random_bytes(100_000, 1);
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data)), 512, 1024, 4096);
        let mut count = 0;
        while let Some(chunk) = chunker.next_chunk().unwrap() {
            assert!(!chunk.data.is_empty());
            assert!(chunk.data.len() <= 4096);
            count += 1;
        }
        assert!(count >= 25, "expected >= 25 chunks, got {}", count);
    }

    #[test]
    fn test_rabin_shift_invariant_deduplication() {
        // Prepending bytes to a file must resynchronize chunk boundaries and preserve
        // the vast majority of downstream chunks (true shift-invariant FastCDC).
        let original = pseudo_random_bytes(256 * 1024, 777);
        let mut shifted = Vec::with_capacity(original.len() + 137);
        shifted.extend_from_slice(&pseudo_random_bytes(137, 12345));
        shifted.extend_from_slice(&original);

        let mut c1 = Chunker::new_rabin(Box::new(Cursor::new(original)), 1024, 4096, 16384);
        let mut hashes1 = HashSet::new();
        while let Some(c) = c1.next_chunk().unwrap() {
            hashes1.insert(c.hash);
        }

        let mut c2 = Chunker::new_rabin(Box::new(Cursor::new(shifted)), 1024, 4096, 16384);
        let mut total2 = 0usize;
        let mut matched = 0usize;
        while let Some(c) = c2.next_chunk().unwrap() {
            total2 += 1;
            if hashes1.contains(&c.hash) {
                matched += 1;
            }
        }

        let dedup_ratio = matched as f64 / total2 as f64;
        assert!(
            dedup_ratio >= 0.80,
            "FastCDC shift-invariant dedup ratio too low: {:.2} ({}/{})",
            dedup_ratio,
            matched,
            total2
        );
    }

    #[test]
    fn test_rabin_chunking_deterministic() {
        let data = pseudo_random_bytes(50_000, 42);
        let mut c1 = Chunker::new_rabin(Box::new(Cursor::new(data.clone())), 512, 1024, 4096);
        let mut c2 = Chunker::new_rabin(Box::new(Cursor::new(data)), 512, 1024, 4096);
        loop {
            let a = c1.next_chunk().unwrap();
            let b = c2.next_chunk().unwrap();
            match (a, b) {
                (None, None) => break,
                (Some(chunk_a), Some(chunk_b)) => {
                    assert_eq!(chunk_a.data, chunk_b.data);
                    assert_eq!(chunk_a.offset, chunk_b.offset);
                }
                _ => panic!("mismatched chunk count"),
            }
        }
    }

    #[test]
    fn test_rabin_integral_roundtrip() {
        let data = pseudo_random_bytes(100_000, 7);
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data.clone())), 512, 1024, 4096);
        let mut assembled = Vec::new();
        let mut expected_offset = 0u64;
        while let Some(chunk) = chunker.next_chunk().unwrap() {
            assert_eq!(chunk.offset, expected_offset);
            expected_offset += chunk.data.len() as u64;
            assembled.extend_from_slice(&chunk.data);
        }
        assert_eq!(assembled, data);
    }

    #[test]
    fn test_chunker_mode_from_config_defaults_fixed() {
        let config = BTreeMap::new();
        assert_eq!(
            ChunkerMode::from_config(&config),
            ChunkerMode::Fixed {
                chunk_size: 4_194_304
            }
        );
    }

    #[test]
    fn test_chunker_mode_from_config_fixed() {
        let mut config = BTreeMap::new();
        config.insert("chunker_mode".to_string(), "fixed".to_string());
        config.insert("chunk_size".to_string(), "1048576".to_string());
        assert_eq!(
            ChunkerMode::from_config(&config),
            ChunkerMode::Fixed {
                chunk_size: 1_048_576
            }
        );
    }

    #[test]
    fn test_chunker_mode_from_config_fastcdc_and_legacy_rabin() {
        for mode in ["fastcdc", "rabin"] {
            let mut config = BTreeMap::new();
            config.insert("chunker_mode".to_string(), mode.to_string());
            config.insert("chunk_min".to_string(), "262144".to_string());
            config.insert("chunk_avg".to_string(), "524288".to_string());
            config.insert("chunk_max".to_string(), "1048576".to_string());
            assert_eq!(
                ChunkerMode::from_config(&config),
                ChunkerMode::FastCdc {
                    min: 262_144,
                    avg: 524_288,
                    max: 1_048_576
                }
            );
        }
    }

    #[test]
    fn test_validate_chunk_size_valid() {
        assert!(validate_chunk_size(1).is_ok());
        assert!(validate_chunk_size(4 * 1024 * 1024).is_ok());
        assert!(validate_chunk_size(16 * 1024 * 1024).is_ok());
    }

    #[test]
    fn test_validate_chunk_size_invalid() {
        assert!(validate_chunk_size(0).is_err());
        assert!(validate_chunk_size(16 * 1024 * 1024 + 1).is_err());
    }

    #[test]
    fn test_validate_rabin_params() {
        assert!(validate_rabin_params(1, 2, 4).is_ok());
        assert!(validate_rabin_params(0, 2, 4).is_err());
        assert!(validate_rabin_params(10, 5, 20).is_err());
        assert!(validate_rabin_params(5, 30, 20).is_err());
        assert!(validate_rabin_params(5, 10, MAX_CHUNK_SIZE + 1).is_err());
    }

    #[test]
    fn test_blake3_reference_vector() {
        let hash = blake3::hash(b"");
        assert_eq!(
            hash.to_hex().to_string(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn test_blake3_reference_vector_abc() {
        let hash = blake3::hash(b"abc");
        assert_eq!(
            hash.to_hex().to_string(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
    }

    #[test]
    fn test_blake3_reference_vector_hello() {
        let hash = blake3::hash(b"hello world");
        assert_eq!(
            hash.to_hex().to_string(),
            "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"
        );
    }

    #[test]
    fn test_blake3_large_input_deterministic() {
        let data = vec![0xABu8; 65536];
        let hash = blake3::hash(&data);
        assert_eq!(
            hash.to_hex().to_string(),
            "b01eb9000c0964964671d09cab928d484b6381cdd0bc1e78f5e788d560bb7984"
        );
    }

    #[test]
    fn test_blake3_keyed_hash() {
        let key = [0u8; 32];
        let mut hasher = blake3::Hasher::new_keyed(&key);
        let hash = hasher.update(b"test data").finalize();
        assert_eq!(hash.to_hex().to_string().len(), 64);
    }
}
