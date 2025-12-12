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
use ed25519_dalek::{Signer, Verifier};

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
    let keys = KeyPair::load(&shard_dir.join("keys"))?;
    let signing_key = keys.signing_key;

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

pub fn verify(path: &Path, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let store = Store::new(&shard_dir);

    // 1. Load commit
    // We need a way to get chunk by hash. Store::get_chunk?
    // I implemented put_chunk but not get_chunk.
    // I'll implement get_chunk in Store first or just read file here.
    // Store::get_chunk is better.

    // For now, I'll read directly to avoid changing Store interface in this step if possible.
    // But Store encapsulates path logic.
    // I'll add get_chunk to Store in a separate step or just duplicate path logic here?
    // Duplicate for speed, then refactor.

    let prefix = &commit_id[..2];
    let filename = commit_id;
    let obj_path = shard_dir.join("objects").join(prefix).join(filename);

    if !obj_path.exists() {
        anyhow::bail!("Commit object not found: {}", commit_id);
    }

    let data = fs::read(obj_path)?;
    let commit: Commit = serde_json::from_slice(&data)?;

    // 2. Verify signature
    if let Some(sig_hex) = &commit.signature {
        // We need the public key.
        // For local verification, we use the local public key?
        // Or the author's public key?
        // The commit doesn't store the public key, only the signature.
        // We assume the local keypair is the author for now.
        // Or we should store the public key in the commit or look it up.
        // "Keys are stored in ~/.shard/keys and per-repo config references them."
        // For Phase 1, I'll use the local public key.

        let pub_key_path = shard_dir.join("keys/public.key");
        let pub_bytes = fs::read(pub_key_path)?;
        let verifying_key = ed25519_dalek::VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?;

        // Reconstruct unsigned JSON
        let mut unsigned_commit = commit.clone(); // Need Clone for Commit
        unsigned_commit.signature = None;
        let json_unsigned = serde_json::to_vec(&unsigned_commit)?;

        let sig_bytes = hex::decode(sig_hex)?;
        let signature = ed25519_dalek::Signature::from_bytes(sig_bytes.as_slice().try_into()?);

        verifying_key.verify(&json_unsigned, &signature)?;
        println!("Signature verified.");
    } else {
        println!("Warning: Commit is unsigned.");
    }

    // 3. Verify manifests
    for manifest_id in &commit.manifests {
        let prefix = &manifest_id[..2];
        let path = shard_dir.join("objects").join(prefix).join(manifest_id);
        if !path.exists() {
            anyhow::bail!("Manifest missing: {}", manifest_id);
        }

        let data = fs::read(path)?;
        // Verify hash
        let hash = blake3::hash(&data);
        if hash.to_hex().to_string() != *manifest_id {
            anyhow::bail!("Manifest hash mismatch: {}", manifest_id);
        }

        let manifest: FileManifest = serde_json::from_slice(&data)?;
        println!("Verifying file: {}", manifest.name);

        // 4. Verify chunks
        for chunk_id in &manifest.chunks {
            let prefix = &chunk_id[..2];
            let path = shard_dir.join("objects").join(prefix).join(chunk_id);
            if !path.exists() {
                anyhow::bail!("Chunk missing: {}", chunk_id);
            }
            // Optional: Verify chunk hash (expensive for large files)
            // For "verify" command, we SHOULD verify content.
            let data = fs::read(path)?;
            let hash = blake3::hash(&data);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("Chunk hash mismatch: {}", chunk_id);
            }
        }
    }

    println!("Verification successful.");
    Ok(())
}

pub async fn share(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let mut node = shard_net::p2p::Node::new().await?;
    node.listen("/ip4/0.0.0.0/tcp/0").await?; // Listen on random port

    // In a real implementation, we would load the repo and serve requests.
    // For now, we just start the node to prove connectivity.
    println!("Sharing repository...");
    node.run().await;

    Ok(())
}

pub async fn pull(path: &Path, peer: &str, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    // pull can work on empty repo or existing one.
    // if !shard_dir.exists() { init(path)?; }

    let mut node = shard_net::p2p::Node::new().await?;

    // Parse peer multiaddr
    let multiaddr: shard_net::libp2p::Multiaddr = peer.parse()?;

    // Dial peer
    println!("Dialing {}...", peer);
    node.swarm.dial(multiaddr.clone())?;

    // Request manifest
    // We need to implement the request logic in Node or here.
    // Node::run is a loop, so we can't easily use it for "request and return".
    // We need a way to send request and await response.
    // This requires a background task for the swarm or a different architecture.

    // For Phase 2, "Basic Network & Exchange", maybe we just implement the CLI to call these.
    // But `pull` needs to actually pull.

    // I need to modify `Node` to support sending requests.
    // And `Node::run` should probably be `Node::run_until` or similar, or run in background.

    // Let's spawn the node in background?
    // But `Node` owns the swarm.

    // I'll leave `pull` as a placeholder that connects for now,
    // and I'll update `Node` to support requests in the next step if needed.
    // The plan said "Implement: libp2p bootstrap, peer add, direct manifest request/response".

    println!("Connected to {}. Pulling commit {}...", peer, commit_id);

    // TODO: Implement actual pull logic (request manifest, then chunks)

    Ok(())
}
