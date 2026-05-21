pub mod chunker;
pub mod commit;
pub mod index;
pub mod manifest;
pub mod store;

use crate::chunker::Chunker;
use crate::commit::Commit;
use crate::index::Index;
use crate::manifest::FileManifest;
use crate::store::Store;
use anyhow::Result;
use ed25519_dalek::{Signer, Verifier};
use serde::Serialize;
use shard_crypto::KeyPair;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn init(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if shard_dir.exists() {
        anyhow::bail!("Shard repository already initialized");
    }
    fs::create_dir_all(shard_dir.join("objects"))?;
    fs::create_dir_all(shard_dir.join("keys"))?;

    let keys = KeyPair::generate();
    keys.save(&shard_dir.join("keys"))?;

    println!(
        "Initialized empty Shard repository in {}",
        shard_dir.display()
    );
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

    let _store = Store::new(&shard_dir);

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
        let verifying_key =
            ed25519_dalek::VerifyingKey::from_bytes(pub_bytes.as_slice().try_into()?)?;

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

fn load_commit(shard_dir: &Path, commit_id: &str) -> Result<Commit> {
    let prefix = &commit_id[..2];
    let path = shard_dir.join("objects").join(prefix).join(commit_id);
    let data = fs::read(path)?;
    let mut commit: Commit = serde_json::from_slice(&data)?;
    commit.commit_id = commit_id.to_string();
    Ok(commit)
}

#[derive(Serialize)]
pub struct LogEntry {
    pub commit_id: String,
    pub parents: Vec<String>,
    pub manifests: Vec<String>,
    pub author: String,
    pub message: String,
    pub timestamp: u64,
    pub signature: Option<String>,
}

impl From<Commit> for LogEntry {
    fn from(c: Commit) -> Self {
        LogEntry {
            commit_id: c.commit_id,
            parents: c.parents,
            manifests: c.manifests,
            author: c.author,
            message: c.message,
            timestamp: c.timestamp,
            signature: c.signature,
        }
    }
}

pub fn log_cmd(path: &Path, json: bool) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let head_path = shard_dir.join("HEAD");
    if !head_path.exists() {
        anyhow::bail!("No commits yet");
    }

    let head = fs::read_to_string(&head_path)?;
    let head = head.trim();

    let mut entries: Vec<LogEntry> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![head.to_string()];

    while let Some(cid) = stack.pop() {
        if !seen.insert(cid.clone()) {
            continue;
        }
        let commit = load_commit(&shard_dir, &cid)?;
        for parent in &commit.parents {
            stack.push(parent.clone());
        }
        entries.push(commit.into());
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else {
        for entry in &entries {
            let ts = {
                let secs = entry.timestamp as i64;
                let tm = time::OffsetDateTime::from_unix_timestamp(secs)
                    .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
                tm.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_else(|_| entry.timestamp.to_string())
            };
            println!("commit {}", entry.commit_id);
            if !entry.parents.is_empty() {
                println!("parents: {}", entry.parents.join(" "));
            }
            println!("author: {}", entry.author);
            println!("date:   {}", ts);
            println!();
            for line in entry.message.lines() {
                println!("    {}", line);
            }
            println!();
        }
    }

    Ok(())
}

pub fn peer_add(path: &Path, multiaddr: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let peers_path = shard_dir.join("peers.json");
    let mut peers: Vec<String> = if peers_path.exists() {
        let data = fs::read(&peers_path)?;
        serde_json::from_slice(&data)?
    } else {
        Vec::new()
    };

    if !peers.contains(&multiaddr.to_string()) {
        peers.push(multiaddr.to_string());
        let data = serde_json::to_vec(&peers)?;
        fs::write(peers_path, data)?;
        println!("Added peer: {}", multiaddr);
    } else {
        println!("Peer already exists: {}", multiaddr);
    }

    Ok(())
}

fn load_peers(shard_dir: &Path) -> Result<Vec<String>> {
    let peers_path = shard_dir.join("peers.json");
    if peers_path.exists() {
        let data = fs::read(peers_path)?;
        Ok(serde_json::from_slice(&data)?)
    } else {
        Ok(Vec::new())
    }
}

struct RepoProvider {
    store: Store,
}

impl shard_net::p2p::ShardContentProvider for RepoProvider {
    fn get_manifest(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
    fn get_chunk(&self, id: &str) -> Option<Vec<u8>> {
        self.store.get_chunk(id).ok()
    }
}

pub async fn share(path: &Path) -> Result<()> {
    let shard_dir = path.join(".shard");
    if !shard_dir.exists() {
        anyhow::bail!("Not a Shard repository");
    }

    let mut node = shard_net::p2p::Node::new().await?;

    // Bootstrap from peers
    let peers = load_peers(&shard_dir)?;
    for peer in peers {
        if let Ok(addr) = peer.parse::<shard_net::libp2p::Multiaddr>() {
            let _ = node.swarm.dial(addr);
        }
    }

    node.listen("/ip4/0.0.0.0/tcp/0").await?; // Listen on random port (TCP)

    // In a real implementation, we would load the repo and serve requests.
    // For now, we just start the node to prove connectivity.
    println!("Sharing repository...");
    let store = Store::new(&shard_dir);
    let provider = RepoProvider { store };
    node.run(provider).await;

    Ok(())
}

pub async fn pull(path: &Path, peer: &str, commit_id: &str) -> Result<()> {
    let shard_dir = path.join(".shard");
    // pull can work on empty repo or existing one.
    // if !shard_dir.exists() { init(path)?; }

    if !shard_dir.exists() {
        init(path)?;
    }

    let store = Store::new(&shard_dir);

    let mut node = shard_net::p2p::Node::new().await?;

    // Parse peer multiaddr
    let multiaddr: shard_net::libp2p::Multiaddr = peer.parse()?;
    let peer_id = match multiaddr.iter().last() {
        Some(shard_net::libp2p::multiaddr::Protocol::P2p(peer_id)) => peer_id,
        _ => anyhow::bail!("Multiaddr must end with /p2p/<peer_id>"),
    };

    // 1. Get Commit (dial + wait + request in one event loop)
    println!("Pulling commit {} from {}...", commit_id, peer);
    let commit_data = node
        .request_manifest(&multiaddr, peer_id, commit_id.to_string())
        .await?;
    // Verify hash
    let hash = blake3::hash(&commit_data);
    if hash.to_hex().to_string() != commit_id {
        anyhow::bail!("Commit hash mismatch");
    }
    // Store commit
    let chunk = crate::chunker::Chunk {
        hash,
        data: commit_data.clone(),
        offset: 0,
    };
    store.put_chunk(&chunk)?;

    let commit: Commit = serde_json::from_slice(&commit_data)?;
    println!("Got commit: {}", commit.message);

    // 2. Get Manifests
    for manifest_id in commit.manifests {
        println!("Fetching manifest {}...", manifest_id);
        let manifest_data = node
            .request_manifest(&multiaddr, peer_id, manifest_id.clone())
            .await?;

        let hash = blake3::hash(&manifest_data);
        if hash.to_hex().to_string() != manifest_id {
            anyhow::bail!("Manifest hash mismatch");
        }

        let chunk = crate::chunker::Chunk {
            hash,
            data: manifest_data.clone(),
            offset: 0,
        };
        store.put_chunk(&chunk)?;

        let manifest: FileManifest = serde_json::from_slice(&manifest_data)?;
        println!("Fetching file: {}", manifest.name);

        // 3. Get Chunks
        for chunk_id in &manifest.chunks {
            // Check if we already have it
            if store.get_chunk(chunk_id).is_ok() {
                continue;
            }

            let chunk_data = node
                .request_chunk(&multiaddr, peer_id, chunk_id.clone())
                .await?;
            let hash = blake3::hash(&chunk_data);
            if hash.to_hex().to_string() != *chunk_id {
                anyhow::bail!("Chunk hash mismatch");
            }

            let chunk = crate::chunker::Chunk {
                hash,
                data: chunk_data,
                offset: 0,
            };
            store.put_chunk(&chunk)?;
        }

        // 4. Reconstruct file
        let mut file_data = Vec::new();
        for chunk_id in &manifest.chunks {
            let data = store.get_chunk(chunk_id)?;
            file_data.extend_from_slice(&data);
        }
        fs::write(path.join(&manifest.name), file_data)?;
        println!("Reconstructed file: {}", manifest.name);
    }

    println!("Pull complete.");
    Ok(())
}
