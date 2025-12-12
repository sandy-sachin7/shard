use anyhow::Result;
use std::io::Read;
use blake3::Hasher;

pub const CHUNK_SIZE: usize = 4 * 1024 * 1024; // 4 MiB

#[derive(Debug, Clone)]
pub struct Chunk {
    pub hash: blake3::Hash,
    pub data: Vec<u8>,
    pub offset: u64,
}

pub struct Chunker<R: Read> {
    reader: R,
    offset: u64,
}

impl<R: Read> Chunker<R> {
    pub fn new(reader: R) -> Self {
        Self { reader, offset: 0 }
    }

    pub fn next_chunk(&mut self) -> Result<Option<Chunk>> {
        let mut buffer = vec![0u8; CHUNK_SIZE];
        let mut bytes_read = 0;

        // Read until buffer is full or EOF
        while bytes_read < CHUNK_SIZE {
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
