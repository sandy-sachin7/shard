pub mod chunker;
pub mod store;
pub mod manifest;
pub mod index;

use std::path::Path;
use anyhow::Result;
use std::fs;
use shard_crypto::KeyPair;
use crate::chunker::Chunker;
use crate::store::Store;
use crate::manifest::FileManifest;
use crate::index::Index;

pub fn init(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!("Shard repository already initialized");
    }
    fs::create_dir_all(shard_dir.join("objects"))?;
    fs::create_dir_all(shard_dir.join("keys"))?;

    let keys = KeyPair::generate();
    keys.save(&shard_dir.join("keys"))?;

    println!("Initialized empty Shard repository in {}", shard_dir.display());
    Ok(())
}

pub fn add(path: &Path, file_path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let mut index = Index::load(&shard_dir.join("index"))?;

    let file = fs::File::open(file_path)?;
    let mut chunker = Chunker::new(file);
    let mut chunk_hashes = Vec::new();
    let mut total_size = 0;

    while let Some(chunk) = chunker.next_chunk()? {
        store.put_chunk(&chunk)?;
        chunk_hashes.push(chunk.hash.to_hex().to_string());
        total_size += chunk.data.len() as u64;
    }

    let filename = file_path.file_name().unwrap().to_string_lossy().to_string();
    let manifest = FileManifest {
        name: filename.clone(),
        size: total_size,
        chunks: chunk_hashes,
        content_type: None,
    };

    index.files.insert(filename.clone(), manifest);
    index.save(&shard_dir.join("index"))?;

    println!("Added {} ({})", filename, total_size);
    Ok(())
}
