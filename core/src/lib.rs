pub mod chunker;
pub mod store;
pub mod manifest;
pub mod index;
pub mod commit;

use std::path::Path;
use anyhow::Result;
use std::fs;
use shard_crypto::KeyPair;
use crate::chunker::Chunker;
use crate::store::Store;
use crate::manifest::FileManifest;
use crate::index::Index;
use crate::commit::Commit;
use std::time::{SystemTime, UNIX_EPOCH};
use ed25519_dalek::Signer;

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

pub fn commit(path: &Path, message: &str, author: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);
    let index = Index::load(&shard_dir.join("index"))?;

    if index.files.is_empty() {
        anyhow::bail!("Nothing to commit");
    }

    // 1. Store manifests
    let mut manifest_ids = Vec::new();
    for manifest in index.files.values() {
        // Canonical JSON
        let json = serde_json::to_vec(manifest)?;
        let hash = blake3::hash(&json);
        let chunk = crate::chunker::Chunk {
            hash,
            data: json,
            offset: 0,
        };
        store.put_chunk(&chunk)?;
        manifest_ids.push(hash.to_hex().to_string());
    }
    manifest_ids.sort(); // Canonical order

    // 2. Get parent
    let head_path = shard_dir.join("HEAD");
    let mut parents = Vec::new();
    if head_path.exists() {
        let head = fs::read_to_string(&head_path)?;
        parents.push(head.trim().to_string());
    }

    // 3. Create commit
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut commit = Commit {
        commit_id: String::new(), // Placeholder
        parents,
        manifests: manifest_ids,
        author: author.to_string(),
        message: message.to_string(),
        timestamp,
        signature: None,
    };

    // 4. Sign
    // Load keys
    // For now, we need to load keys. KeyPair needs a load method.
    // I'll implement a quick load here or assume keys exist.
    // I need to update KeyPair to support loading.
    // For now, I'll just skip signing or try to load raw bytes.

    let secret_key_path = shard_dir.join("keys/secret.key");
    let secret_bytes = fs::read(secret_key_path)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(secret_bytes.as_slice().try_into()?);

    // Canonical JSON for signing (without signature)
    let json_unsigned = serde_json::to_vec(&commit)?;
    let signature = signing_key.sign(&json_unsigned);
    commit.signature = Some(hex::encode(signature.to_bytes()));

    // 5. Store commit
    let json_final = serde_json::to_vec(&commit)?;
    let hash = blake3::hash(&json_final);
    let chunk = crate::chunker::Chunk {
        hash,
        data: json_final,
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    // 6. Update HEAD
    let commit_id = hash.to_hex().to_string();
    fs::write(head_path, &commit_id)?;

    println!("Committed {} ({})", commit_id, message);
    Ok(())
}
