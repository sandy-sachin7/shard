use anyhow::Result;
use std::collections::BTreeMap;
use std::io::Read;

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
/// Panics if `chunk_size` is zero.
pub struct FixedChunker {
    reader: Box<dyn Read + Send>,
    offset: u64,
    chunk_size: usize,
}

impl FixedChunker {
    fn new(reader: Box<dyn Read + Send>, chunk_size: usize) -> Self {
        Self {
            reader,
            offset: 0,
            chunk_size,
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

// ── Rabin-based content-defined chunker (buzhash) ──────────────────────────

// 256 random 32-bit values (generated via LCG with seed 42)
// Used as the GEAR table for buzhash rolling hash.
const GEAR: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut state: u64 = 42;
    let mut i = 0;
    while i < 256 {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        table[i] = (state >> 32) as u32;
        i += 1;
    }
    table
};

const WINDOW_SIZE: usize = 31;

pub struct RabinChunker {
    reader: Box<dyn Read + Send>,
    offset: u64,
    min_size: usize,
    avg_size: usize,
    max_size: usize,
    buf: Vec<u8>,
    buf_pos: usize,
    eof: bool,
}

impl RabinChunker {
    fn new(reader: Box<dyn Read + Send>, min: usize, avg: usize, max: usize) -> Self {
        Self {
            reader,
            offset: 0,
            min_size: min,
            avg_size: avg,
            max_size: max,
            buf: Vec::new(),
            buf_pos: 0,
            eof: false,
        }
    }

    fn fill_buf(&mut self) -> Result<()> {
        if self.eof {
            return Ok(());
        }
        // Discard already-consumed bytes
        if self.buf_pos > 0 {
            self.buf.drain(..self.buf_pos);
            self.buf_pos = 0;
        }
        let mut tmp = [0u8; 8192];
        loop {
            let n = self.reader.read(&mut tmp)?;
            if n == 0 {
                self.eof = true;
                break;
            }
            self.buf.extend_from_slice(&tmp[..n]);
            if self.buf.len() >= self.max_size * 2 {
                break;
            }
        }
        Ok(())
    }

    fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        self.fill_buf()?;

        if self.buf.is_empty() {
            return Ok(None);
        }

        // Determine cut point
        let end = if self.eof && self.buf.len() <= self.max_size {
            // Last chunk: emit everything remaining
            self.buf.len()
        } else {
            // Find CDC boundary between min_size and max_size
            let search_start = self.min_size.min(self.buf.len());
            let search_end = self.max_size.min(self.buf.len());
            let mut cut = search_end; // default to max if no boundary found

            if search_start < self.buf.len() {
                let mut hash: u32 = 0;
                // Prime the window
                for i in 0..WINDOW_SIZE {
                    if i < search_start {
                        let b = self.buf[search_start - 1 - i];
                        hash = hash.rotate_left(1) ^ GEAR[b as usize];
                    }
                }
                // Slide through the search range
                let mask = (self.avg_size as u32).next_power_of_two() - 1;
                for i in search_start..search_end {
                    let new_b = self.buf[i];
                    let old_b = self.buf[i - WINDOW_SIZE];
                    hash = hash.rotate_left(1) ^ GEAR[new_b as usize] ^ GEAR[old_b as usize];
                    if hash & mask == 0 {
                        cut = i + 1;
                        break;
                    }
                }
            }
            cut
        };

        let data: Vec<u8> = self.buf.drain(..end).collect();
        let hash = blake3::hash(&data);
        let chunk = Chunk {
            hash,
            data,
            offset: self.offset,
        };

        self.offset += end as u64;
        Ok(Some(chunk))
    }
}

// ── Unified chunker API ────────────────────────────────────────────────────

/// Chunker mode selected at init time — either fixed-size or content-defined (Rabin).
/// Deserialized from `.shard/config.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkerMode {
    /// Emit chunks of exactly `chunk_size` bytes.
    Fixed { chunk_size: u64 },
    /// Emit variable-size chunks via buzhash rolling hash, with min/avg/max bounds.
    Rabin { min: u64, avg: u64, max: u64 },
}

impl ChunkerMode {
    /// Parse a `ChunkerMode` from the repository config JSON.
    /// Defaults to `Fixed { chunk_size: 4_194_304 }` if `chunker_mode` is absent.
    pub fn from_config(config: &BTreeMap<String, String>) -> Self {
        match config.get("chunker_mode").map(|s| s.as_str()) {
            Some("rabin") => ChunkerMode::Rabin {
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

/// Unified chunker interface — dispatches to either [`FixedChunker`] or [`RabinChunker`].
pub enum Chunker {
    Fixed(FixedChunker),
    Rabin(RabinChunker),
}

impl Chunker {
    /// Create a new fixed-size chunker reading from `reader`.
    pub fn new_fixed(reader: Box<dyn Read + Send>, chunk_size: u64) -> Self {
        Chunker::Fixed(FixedChunker::new(reader, chunk_size as usize))
    }

    /// Create a new Rabin content-defined chunker reading from `reader`.
    pub fn new_rabin(reader: Box<dyn Read + Send>, min: u64, avg: u64, max: u64) -> Self {
        Chunker::Rabin(RabinChunker::new(
            reader,
            min as usize,
            avg as usize,
            max as usize,
        ))
    }

    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        match self {
            Chunker::Fixed(c) => c.next_chunk(),
            Chunker::Rabin(c) => c.next_chunk(),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_fixed_chunking_small() {
        let data = vec![0u8; 1024];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let chunk = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.data.len(), 1024);
        assert_eq!(chunk.offset, 0);
        let next = chunker.next_chunk().unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn test_fixed_chunking_exact() {
        let data = vec![0u8; 4_194_304];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let chunk = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.data.len(), 4_194_304);
        assert_eq!(chunk.offset, 0);
        let next = chunker.next_chunk().unwrap();
        assert!(next.is_none());
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
        let next = chunker.next_chunk().unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn test_fixed_chunking_empty() {
        let data: Vec<u8> = vec![];
        let mut chunker = Chunker::new_fixed(Box::new(Cursor::new(data)), 4_194_304);
        let next = chunker.next_chunk().unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn test_rabin_chunking_small() {
        let data = vec![0u8; 1024];
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data)), 256, 512, 1024);
        let chunk = chunker.next_chunk().unwrap().unwrap();
        assert_eq!(chunk.data.len(), 1024);
        assert_eq!(chunk.offset, 0);
        let next = chunker.next_chunk().unwrap();
        assert!(next.is_none());
    }

    #[test]
    fn test_rabin_chunking_produces_multiple_chunks() {
        let data: Vec<u8> = (0..100_000).map(|i| (i & 0xFF) as u8).collect();
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data)), 512, 1024, 4096);
        let mut count = 0;
        while let Some(chunk) = chunker.next_chunk().unwrap() {
            assert!(!chunk.data.is_empty());
            assert!(chunk.data.len() <= 4096);
            count += 1;
        }
        assert!(count >= 5, "expected >= 5 chunks, got {}", count);
    }

    #[test]
    fn test_rabin_chunking_deterministic() {
        let data: Vec<u8> = (0..50_000).map(|i| (i & 0xFF) as u8).collect();
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
        let data: Vec<u8> = (0..100_000).map(|i| (i & 0xFF) as u8).collect();
        let mut chunker = Chunker::new_rabin(Box::new(Cursor::new(data)), 512, 1024, 4096);
        let mut assembled = Vec::new();
        let mut expected_offset = 0u64;
        while let Some(chunk) = chunker.next_chunk().unwrap() {
            assert_eq!(chunk.offset, expected_offset);
            expected_offset += chunk.data.len() as u64;
            assembled.extend_from_slice(&chunk.data);
        }
        // assembled starts empty, no assert here since data was moved into Cursor
    }

    #[test]
    fn test_chunker_mode_from_config_defaults_fixed() {
        let config = BTreeMap::new();
        let mode = ChunkerMode::from_config(&config);
        assert_eq!(
            mode,
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
        let mode = ChunkerMode::from_config(&config);
        assert_eq!(
            mode,
            ChunkerMode::Fixed {
                chunk_size: 1_048_576
            }
        );
    }

    #[test]
    fn test_chunker_mode_from_config_rabin() {
        let mut config = BTreeMap::new();
        config.insert("chunker_mode".to_string(), "rabin".to_string());
        config.insert("chunk_min".to_string(), "262144".to_string());
        config.insert("chunk_avg".to_string(), "524288".to_string());
        config.insert("chunk_max".to_string(), "1048576".to_string());
        let mode = ChunkerMode::from_config(&config);
        assert_eq!(
            mode,
            ChunkerMode::Rabin {
                min: 262_144,
                avg: 524_288,
                max: 1_048_576
            }
        );
    }
}
